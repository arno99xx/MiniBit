use valence::{interact_block::InteractBlockEvent, inventory::HeldItem, prelude::*};

#[derive(Resource)]
struct DiggingPluginResource {
    whitelist: Vec<BlockKind>,
}

pub struct DiggingPlugin {
    pub whitelist: Vec<BlockKind>,
}

impl Plugin for DiggingPlugin {
    fn build(&self, app: &mut App) {
        app
            .insert_resource(DiggingPluginResource {
                whitelist: self.whitelist.clone(),
            })
            .add_systems(Update, handle_digging_events);
    }
}

fn handle_digging_events(
    mut clients: Query<(&GameMode, &mut Inventory, &VisibleChunkLayer)>,
    mut layers: Query<&mut ChunkLayer>,
    mut events: EventReader<DiggingEvent>,
    res: Res<DiggingPluginResource>,
) {
    for event in events.read() {
        if let Ok((gamemode, mut inv, layer)) = clients.get_mut(event.client) {
            if *gamemode == GameMode::Adventure || *gamemode == GameMode::Spectator || event.state != DiggingState::Stop {
                continue;
            }
            let Ok(mut chunk_layer) = layers.get_mut(layer.0) else {
                continue;
            };
            let Some(block) = chunk_layer.block(event.position) else {
                continue;
            };
            let kind = block.state.to_kind();
            if res.whitelist.contains(&kind) {
                let kind = kind.to_item_kind();
                if let Some(slot) = inv.first_slot_with_item(kind, 64) {
                    let count = inv.slot(slot).count + 1;
                    inv.set_slot_amount(slot, count);
                } else if let Some(slot) = inv.first_empty_slot_in(9..45) {
                    inv.set_slot(slot, ItemStack::new(kind, 1, None));
                }
                chunk_layer.set_block(event.position, BlockState::AIR);
            }
        }
    }
}

pub struct PlacingPlugin;

impl Plugin for PlacingPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, handle_placing_events);
    }
}

fn handle_placing_events(
    mut clients: Query<(&GameMode, &mut Inventory, &HeldItem, &VisibleChunkLayer)>,
    mut layers: Query<&mut ChunkLayer>,
    mut events: EventReader<InteractBlockEvent>,
) {
    for event in events.read() {
        if let Ok((gamemode, mut inv, held_item, layer)) = clients.get_mut(event.client) {
            if *gamemode == GameMode::Adventure || *gamemode == GameMode::Spectator || event.hand != Hand::Main {
                continue;
            }
            let Ok(mut chunk_layer) = layers.get_mut(layer.0) else {
                continue;
            };
            let slot = held_item.slot();
            if inv.slot(slot).count == 0 {
                continue;
            }
            let kind = inv.slot(slot).item;
            if let Some(block_kind) = BlockKind::from_item_kind(kind) {
                let pos = event.position.get_in_direction(event.face);
                let Some(block) = chunk_layer.block(pos) else {
                    continue;
                };
                if block.state == BlockState::AIR {
                    chunk_layer.set_block(pos, block_kind.to_state());
                    let count = inv.slot(slot).count - 1;
                    inv.set_slot_amount(slot, count);
                }
            }
        }
    }
}
