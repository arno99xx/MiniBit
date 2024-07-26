#![allow(clippy::type_complexity)]

#[path = "../lib/mod.rs"]
mod lib;

use bevy_ecs::query::WorldQuery;
use lib::duels::*;
use lib::physics::*;
use valence::entity::arrow::ArrowEntityBundle;
use valence::entity::living::Health;
use valence::entity::Velocity;
use valence::entity::{EntityId, EntityStatuses};
use valence::event_loop::PacketEvent;
use valence::inventory::PlayerAction;
use valence::math::Vec3Swizzles;
use valence::prelude::*;
use valence::protocol::packets::play::DamageTiltS2c;
use valence::protocol::packets::play::PlayerActionC2s;
use valence::protocol::sound::SoundCategory;
use valence::protocol::Sound;
use valence::protocol::VarInt;
use valence::protocol::WritePacket;

#[derive(Component)]
struct ProjectileOwner(Entity);

fn main() {
    App::new()
        .add_plugins(DuelsPlugin { default_gamemode: GameMode::Adventure })
        .add_plugins(DefaultPlugins)
        .add_plugins(ProjectilePlugin)
        .add_systems(
            EventLoopUpdate,
            (handle_combat_events, handle_player_action),
        )
        .add_systems(
            Update,
            (
                gamestage_change.after(lib::duels::gameloop),
                end_game.after(lib::duels::end_game),
                handle_collision_events,
                handle_oob_clients,
                calc_player_vel,
            ),
        )
        .run();
}

fn gamestage_change(
    mut clients: Query<&mut Inventory, With<Client>>,
    games: Query<&Entities>,
    mut event: EventReader<GameStageEvent>,
) {
    for event in event.read() {
        if event.stage != 4 {
            continue;
        }
        if let Ok(entities) = games.get(event.game_id) {
            for entity in entities.0.iter() {
                if let Ok(mut inventory) = clients.get_mut(*entity) {
                    inventory.set_slot(36, ItemStack::new(ItemKind::Bow, 1, None));
                    inventory.set_slot(44, ItemStack::new(ItemKind::Arrow, 10, None));
                }
            }
        }
    }
}

fn end_game(
    mut clients: Query<&mut Inventory, With<Client>>,
    games: Query<&Entities>,
    mut end_game: EventReader<EndGameEvent>,
) {
    for event in end_game.read() {
        if let Ok(entities) = games.get(event.game_id) {
            for entity in entities.0.iter() {
                if let Ok(mut inv) = clients.get_mut(*entity) {
                    for slot in 0..inv.slot_count() {
                        inv.set_slot(slot, ItemStack::EMPTY);
                    }
                }
            }
        }
    }
}

#[derive(WorldQuery)]
#[world_query(mutable)]
struct CombatQuery {
    client: &'static mut Client,
    id: &'static EntityId,
    pos: &'static Position,
    vel: &'static Velocity,
    state: &'static mut CombatState,
    statuses: &'static mut EntityStatuses,
    gamestate: &'static PlayerGameState,
    health: &'static mut Health,
}

fn handle_combat_events(
    server: Res<Server>,
    mut clients: Query<CombatQuery>,
    mut sprinting: EventReader<SprintEvent>,
    mut interact_entity: EventReader<InteractEntityEvent>,
    mut end_game: EventWriter<EndGameEvent>,
) {
    for &SprintEvent { client, state } in sprinting.read() {
        if let Ok(mut client) = clients.get_mut(client) {
            client.state.has_bonus_knockback = state == SprintState::Start;
        }
    }

    for &InteractEntityEvent {
        client: attacker_client,
        entity: victim_client,
        interact: interaction,
        ..
    } in interact_entity.read()
    {
        let Ok([mut attacker, mut victim]) = clients.get_many_mut([attacker_client, victim_client])
        else {
            continue;
        };

        if interaction != EntityInteraction::Attack
            || server.current_tick() - victim.state.last_attacked_tick < 10
            || attacker.gamestate.game_id != victim.gamestate.game_id
        {
            continue;
        }

        victim.state.last_attacked_tick = server.current_tick();

        let victim_pos = victim.pos.0.xz();
        let attacker_pos = attacker.pos.0.xz();

        let dir = (victim_pos - attacker_pos).normalize().as_vec2();

        let knockback_xz = if attacker.state.has_bonus_knockback {
            18.0
        } else {
            8.0
        };
        let knockback_y = if attacker.state.has_bonus_knockback {
            8.432
        } else {
            6.432
        };

        damage_player(
            &mut attacker,
            &mut victim,
            1.0,
            Vec3::new(dir.x * knockback_xz, knockback_y, dir.y * knockback_xz),
            &mut end_game,
        );

        attacker.state.has_bonus_knockback = false;
    }
}

#[derive(WorldQuery)]
#[world_query(mutable)]
struct ActionQuery {
    entity: Entity,
    inv: &'static mut Inventory,
    pos: &'static Position,
    look: &'static Look,
    yaw: &'static HeadYaw,
    layer: &'static EntityLayerId,
    state: &'static mut CombatState,
}
fn handle_player_action(
    mut players: Query<ActionQuery>,
    mut clients: Query<&mut Client>,
    mut packets: EventReader<PacketEvent>,
    mut commands: Commands,
) {
    for packet in packets.read() {
        if let Some(pkt) = packet.decode::<PlayerActionC2s>() {
            let Ok(mut player) = players.get_mut(packet.client) else {
                continue;
            };
            if pkt.action == PlayerAction::ReleaseUseItem
                && player.inv.slot(36).item == ItemKind::Bow
                && player.inv.slot(44).item == ItemKind::Arrow
            {
                let count = player.inv.slot(44).count;
                player.inv.set_slot_amount(44, count - 1);
                for mut client in clients.iter_mut() {
                    client.play_sound(
                        Sound::EntityArrowShoot,
                        SoundCategory::Player,
                        player.pos.0,
                        1.0,
                        1.0,
                    );
                }
                let rad_yaw = player.yaw.0.to_radians();
                let rad_pitch = player.look.pitch.to_radians();
                let hspeed = rad_pitch.cos();
                let vel = Vec3::new(
                    -rad_yaw.sin() * hspeed,
                    -rad_pitch.sin(),
                    rad_yaw.cos() * hspeed,
                ) * 30.0;
                let dir = vel.normalize().as_dvec3() * 0.5;
                let arrow_id = commands
                    .spawn(ArrowEntityBundle {
                        position: Position(DVec3::new(
                            player.pos.0.x + dir.x,
                            player.pos.0.y + 1.62,
                            player.pos.0.z + dir.z,
                        )),
                        look: *player.look,
                        head_yaw: *player.yaw,
                        velocity: Velocity(vel),
                        layer: *player.layer,
                        ..Default::default()
                    })
                    .id();
                commands
                    .entity(arrow_id)
                    .insert(ProjectileOwner(player.entity));
            }
        }
    }
}

fn handle_collision_events(
    mut clients: Query<CombatQuery>,
    arrows: Query<&ProjectileOwner>,
    mut collisions: EventReader<ProjectileCollisionEvent>,
    mut end_game: EventWriter<EndGameEvent>,
) {
    for event in collisions.read() {
        if let Ok(owner) = arrows.get(event.arrow) {
            if let Ok([mut attacker, mut victim]) = clients.get_many_mut([owner.0, event.player]) {
                damage_player(
                    &mut attacker,
                    &mut victim,
                    6.0,
                    Vec3::new(0.0, 0.0, 0.0),
                    &mut end_game,
                );
            }
        }
    }
}

fn handle_oob_clients(
    positions: Query<(&mut Position, &PlayerGameState), With<Client>>,
    mut end_game: EventWriter<EndGameEvent>,
) {
    for (pos, gamestate) in positions.iter() {
        if pos.0.y < 0.0 {
            if gamestate.game_id.is_some() {
                end_game.send(EndGameEvent {
                    game_id: gamestate.game_id.unwrap(),
                    loser: gamestate.team,
                });
            }
        }
    }
}

fn calc_player_vel(mut clients: Query<(&Position, &OldPosition, &mut Velocity), With<Client>>) {
    for (pos, old_pos, mut vel) in clients.iter_mut() {
        vel.0 = Vec3::new(
            (pos.0.x - old_pos.get().x) as f32,
            (pos.0.y - old_pos.get().y) as f32,
            (pos.0.z - old_pos.get().z) as f32,
        );
    }
}

// Helper functions below

fn damage_player(
    attacker: &mut CombatQueryItem,
    victim: &mut CombatQueryItem,
    damage: f32,
    velocity: Vec3,
    end_game: &mut EventWriter<EndGameEvent>,
) {
    victim
        .client
        .set_velocity(victim.vel.0 + velocity);

    attacker.state.has_bonus_knockback = false;

    victim.client.play_sound(
        Sound::EntityPlayerHurt,
        SoundCategory::Player,
        victim.pos.0,
        1.0,
        1.0,
    );
    victim.client.write_packet(&DamageTiltS2c {
        entity_id: VarInt(0),
        yaw: 0.0,
    });
    attacker.client.play_sound(
        Sound::EntityPlayerHurt,
        SoundCategory::Player,
        victim.pos.0,
        1.0,
        1.0,
    );
    attacker.client.write_packet(&DamageTiltS2c {
        entity_id: VarInt(victim.id.get()),
        yaw: 0.0,
    });

    if victim.health.0 <= damage {
        end_game.send(EndGameEvent {
            game_id: victim.gamestate.game_id.unwrap(),
            loser: victim.gamestate.team,
        });
    } else {
        victim.health.0 -= damage;
    }
}