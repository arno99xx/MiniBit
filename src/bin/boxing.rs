#![allow(clippy::type_complexity)]

#[path = "../lib/mod.rs"]
mod lib;

use bevy_ecs::query::WorldQuery;
use lib::config::*;
use lib::game::*;
use valence::entity::{EntityId, EntityStatuses};
use valence::math::Vec3Swizzles;
use valence::prelude::*;
use valence::protocol::packets::play::DamageTiltS2c;
use valence::protocol::sound::SoundCategory;
use valence::protocol::Sound;
use valence::protocol::VarInt;
use valence::protocol::WritePacket;

#[derive(Component, Default)]
struct BoxingState {
    hits: u8,
}

pub fn main() {
    match register_defaults(&mut App::new()) {
        Ok(app) => {
            app.add_systems(EventLoopUpdate, handle_combat_events)
                .add_systems(
                    Update,
                    (
                        init_clients.after(lib::game::init_clients),
                        handle_oob_clients,
                        end_game.after(lib::game::end_game),
                    ),
                )
                .run();
        }
        Err(e) => eprintln!("{}", e),
    }
}

fn init_clients(clients: Query<Entity, Added<Client>>, mut commands: Commands) {
    for client in clients.iter() {
        commands.entity(client).insert(BoxingState::default());
    }
}

#[derive(WorldQuery)]
#[world_query(mutable)]
struct CombatQuery {
    client: &'static mut Client,
    id: &'static EntityId,
    pos: &'static Position,
    state: &'static mut CombatState,
    statuses: &'static mut EntityStatuses,
    gamestate: &'static PlayerGameState,
    boxing_state: &'static mut BoxingState,
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

        victim
            .client
            .set_velocity([dir.x * knockback_xz, knockback_y, dir.y * knockback_xz]);

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

        victim.boxing_state.hits += 1;

        if victim.boxing_state.hits >= 5 {
            victim
                .client
                .send_chat_message("You have been knocked out!");
            attacker
                .client
                .send_chat_message("You have knocked out your opponent!");
            end_game.send(EndGameEvent {
                game_id: victim.gamestate.game_id.unwrap(),
                loser: victim.gamestate.team,
            });
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

fn end_game(
    mut clients: Query<&mut BoxingState>,
    games: Query<&Entities>,
    mut end_game: EventReader<EndGameEvent>,
) {
    for event in end_game.read() {
        let Ok(entities) = games.get(event.game_id) else {
            continue;
        };
        for entity in entities.0.iter() {
            if let Ok(mut state) = clients.get_mut(*entity) {
                state.hits = 0;
            }
        }
    }
}
