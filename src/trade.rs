//! Trading with a village: buying its surplus and selling it what it lacks, for coins.

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    Mode,
    drops::Drop,
    hud::Toast,
    icons::ItemIcons,
    items::{Inventory, Item, Stack},
    player::Player,
    sound::{PlaySound, Sfx},
    ui,
    villages::{GOODS, Village},
};

pub struct TradePlugin;

impl Plugin for TradePlugin {
    fn build(&self, app: &mut App) {
        app.add_client_message::<Deal>(Channel::Ordered)
            .add_server_message::<Dealt>(Channel::Ordered)
            .init_resource::<Market>()
            .add_systems(OnEnter(Mode::Trade), spawn_screen)
            .add_systems(Update, (refresh, close).run_if(in_state(Mode::Trade)))
            .add_systems(Update, settle.run_if(crate::net::authority))
            .add_systems(Update, receive);
    }
}

/// The village whose market is open.
#[derive(Resource, Default)]
pub struct Market(pub u32);

/// A player's offer: a bundle of `GOODS[good]` for `price` coins, already paid for.
#[derive(Message, Serialize, Deserialize)]
struct Deal {
    village: u32,
    good: usize,
    buy: bool,
    price: u32,
}

/// What a player gets from a deal: the goods, or their payment back if it fell through.
#[derive(Message, Serialize, Deserialize)]
struct Dealt(Stack, bool);

fn settle(
    mut deals: MessageReader<FromClient<Deal>>,
    mut villages: Query<&mut Village>,
    mut replies: MessageWriter<ToClients<Dealt>>,
) {
    for deal in deals.read() {
        let Deal { village, good, buy, price } = deal.message;
        let Some(g) = GOODS.get(good) else { continue };
        let Some(mut v) = villages.iter_mut().find(|v| v.id == village) else { continue };
        let (stack, done) = if buy {
            if v.sells(good) == Some(price) && v.take(g.item, g.bundle) {
                v.coins += price as f32;
                (Stack::new(g.item, g.bundle), true)
            } else {
                (Stack::new(Item::Coin, price), false)
            }
        } else if v.buys(good) == Some(price) {
            v.add(g.item, g.bundle);
            v.coins -= price as f32;
            (Stack::new(Item::Coin, price), true)
        } else {
            (Stack::new(g.item, g.bundle), false)
        };
        replies.write(ToClients { targets: SendTargets::Single(deal.client_id), message: Dealt(stack, done) });
    }
}

fn receive(
    mut replies: MessageReader<Dealt>,
    inventory: Option<ResMut<Inventory>>,
    player: Query<&Transform, With<Player>>,
    mut drops: MessageWriter<Drop>,
    mut toast: ResMut<Toast>,
    mut sounds: MessageWriter<PlaySound>,
) {
    let Some(mut inventory) = inventory else { return };
    for Dealt(stack, done) in replies.read() {
        let left = inventory.add_stack(*stack);
        if left > 0
            && let Ok(at) = player.single()
        {
            let stack = Stack { count: left, ..*stack };
            drops.write(Drop { stack, at: at.translation + Vec3::Y, velocity: Vec3::Y, delay: 1.5 });
        }
        if *done {
            sounds.write(PlaySound::ui(Sfx::Pickup));
        } else {
            toast.show("Сделка сорвалась: цены уже другие".into());
        }
    }
}

#[derive(Component)]
struct Board;

fn spawn_screen(mut commands: Commands) {
    let screen = ui::screen(&mut commands, DespawnOnExit(Mode::Trade), Color::srgba(0.0, 0.0, 0.0, 0.6));
    let panel = ui::panel(&mut commands, screen, 560.0);
    commands.entity(panel).insert(Board);
}

/// Redraws the market whenever the village or the purse changes.
fn refresh(
    mut commands: Commands,
    market: Res<Market>,
    villages: Query<Ref<Village>>,
    inventory: Res<Inventory>,
    icons: Res<ItemIcons>,
    board: Single<(Entity, Option<&Children>, Ref<Board>)>,
) {
    let Some(v) = villages.iter().find(|v| v.id == market.0) else { return };
    let (board, children, fresh) = *board;
    if !fresh.is_added() && !v.is_changed() && !inventory.is_changed() {
        return;
    }
    for child in children.into_iter().flatten() {
        commands.entity(*child).despawn();
    }
    let dim = Color::srgba(1.0, 1.0, 1.0, 0.7);
    ui::title(&mut commands, board, &format!("Рынок: {}", v.name), 30.0);
    let purse = inventory.count(Item::Coin);
    ui::label(&mut commands, board, &format!("Казна деревни: {:.0} монет · у вас: {purse}", v.coins), 16.0, dim);
    for (i, g) in GOODS.iter().enumerate() {
        let row = ui::row(&mut commands, board);
        commands.spawn((
            ImageNode::new(icons.get(g.item)),
            Node { width: px(32), height: px(32), ..default() },
            ChildOf(row),
        ));
        let text = format!("{} ×{} · на складе {}", g.item.name(), g.bundle, v.stock[i]);
        let name = ui::label(&mut commands, row, &text, 16.0, Color::WHITE);
        commands.entity(name).insert(Node { flex_grow: 1.0, ..default() });
        if let Some(price) = v.sells(i).filter(|p| *p <= purse) {
            deal_button(
                &mut commands,
                row,
                &format!("Купить: {price}"),
                Deal { village: v.id, good: i, buy: true, price },
            );
        }
        if let Some(price) = v.buys(i).filter(|_| inventory.count(g.item) >= g.bundle) {
            deal_button(
                &mut commands,
                row,
                &format!("Продать: {price}"),
                Deal { village: v.id, good: i, buy: false, price },
            );
        }
    }
    ui::label(&mut commands, board, "Дорого то, чего мало. Esc — закрыть", 14.0, dim);
}

fn deal_button(commands: &mut Commands, row: Entity, text: &str, deal: Deal) {
    let button = ui::button(commands, row, text);
    let Deal { village, good, buy, price } = deal;
    commands.entity(button).insert(Node { width: px(130), ..default() }).observe(
        move |_: On<PointerClick>, mut inventory: ResMut<Inventory>, mut deals: MessageWriter<Deal>| {
            // Paid up front; what comes back is the goods or the payment.
            let (item, n) = if buy { (Item::Coin, price) } else { (GOODS[good].item, GOODS[good].bundle) };
            if inventory.remove(item, n) {
                deals.write(Deal { village, good, buy, price });
            }
        },
    );
}

fn close(keys: Res<ButtonInput<KeyCode>>, mut mode: ResMut<NextState<Mode>>) {
    if keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::KeyE) {
        mode.set(Mode::Playing);
    }
}
