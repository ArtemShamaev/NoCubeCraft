//! Sleeping: lying in bed, and the night passing once every player in the world is asleep.

use std::collections::HashSet;

use bevy::prelude::*;
use bevy_replicon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{AppState, Mode, hud::Toast, net::AvatarOf, sky::TimeOfDay, ui};

pub const MORNING: f32 = 0.0;
/// Everyone has to stay in bed this long before the night passes.
const FALL_ASLEEP_SECS: f32 = 2.0;

pub struct BedsPlugin;

impl Plugin for BedsPlugin {
    fn build(&self, app: &mut App) {
        app.add_client_message::<Sleep>(Channel::Ordered)
            .add_server_message::<Sleepers>(Channel::Ordered)
            .add_server_message::<Morning>(Channel::Ordered)
            .init_resource::<Asleep>()
            .add_systems(OnEnter(Mode::Sleeping), spawn_screen)
            .add_systems(OnExit(Mode::Sleeping), |mut sleep: MessageWriter<Sleep>| {
                sleep.write(Sleep(false));
            })
            .add_systems(Update, (get_up, show_count).run_if(in_state(Mode::Sleeping)))
            .add_systems(Update, wake.run_if(in_state(AppState::Game)))
            .add_systems(Update, count.run_if(in_state(AppState::Game)).run_if(crate::net::authority))
            .add_systems(OnExit(AppState::Game), |mut asleep: ResMut<Asleep>| *asleep = Asleep::default());
    }
}

/// A player got into bed, or out of it.
#[derive(Message, Serialize, Deserialize)]
pub struct Sleep(pub bool);

/// How many players are in bed, of all of them.
#[derive(Message, Serialize, Deserialize, Clone, Copy)]
struct Sleepers {
    asleep: usize,
    total: usize,
}

/// The night has passed: the new time of day.
#[derive(Message, Serialize, Deserialize)]
struct Morning(f32);

/// Where the world is owned: who is in bed, and for how long all of them have been.
#[derive(Resource, Default)]
struct Asleep {
    who: HashSet<ClientId>,
    all_for: f32,
}

fn count(
    time: Res<Time>,
    mut asleep: ResMut<Asleep>,
    mut sleeps: MessageReader<FromClient<Sleep>>,
    guests: Query<&AvatarOf>,
    mut time_of_day: ResMut<TimeOfDay>,
    mut counts: MessageWriter<ToClients<Sleepers>>,
    mut mornings: MessageWriter<ToClients<Morning>>,
) {
    let before = asleep.who.len();
    for sleep in sleeps.read() {
        if sleep.message.0 {
            asleep.who.insert(sleep.client_id);
        } else {
            asleep.who.remove(&sleep.client_id);
        }
    }
    // Guests who left don't keep the night going.
    asleep.who.retain(|id| *id == ClientId::Server || guests.iter().any(|g| g.0 == *id));
    let total = guests.iter().count() + 1;
    if asleep.who.len() != before {
        counts.write(ToClients { targets: SendTargets::All, message: Sleepers { asleep: asleep.who.len(), total } });
    }
    let everyone = !asleep.who.is_empty() && asleep.who.len() == total;
    asleep.all_for = if everyone { asleep.all_for + time.delta_secs() } else { 0.0 };
    let night_over = !asleep.who.is_empty() && !time_of_day.is_night();
    if (everyone && asleep.all_for > FALL_ASLEEP_SECS) || night_over {
        if everyone {
            time_of_day.0 = MORNING;
        }
        asleep.who.clear();
        mornings.write(ToClients { targets: SendTargets::All, message: Morning(time_of_day.0) });
    }
}

fn wake(
    mut mornings: MessageReader<Morning>,
    mode: Option<Res<State<Mode>>>,
    mut next: ResMut<NextState<Mode>>,
    mut time_of_day: ResMut<TimeOfDay>,
    mut toast: ResMut<Toast>,
) {
    let Some(morning) = mornings.read().last() else { return };
    time_of_day.0 = morning.0;
    if mode.is_some_and(|m| **m == Mode::Sleeping) {
        next.set(Mode::Playing);
        toast.show("Доброе утро!".into());
    }
}

#[derive(Component)]
struct CountText;

fn spawn_screen(mut commands: Commands) {
    let screen = ui::screen(&mut commands, DespawnOnExit(Mode::Sleeping), Color::srgba(0.0, 0.0, 0.05, 0.55));
    ui::title(&mut commands, screen, "Вы в кровати", 40.0);
    let count = ui::label(&mut commands, screen, "", 18.0, Color::srgba(1.0, 1.0, 1.0, 0.8));
    commands.entity(count).insert(CountText);
    let panel = ui::panel(&mut commands, screen, 280.0);
    let up = ui::button(&mut commands, panel, "Встать (Esc)");
    commands.entity(up).observe(|_: On<PointerClick>, mut mode: ResMut<NextState<Mode>>| mode.set(Mode::Playing));
}

fn get_up(keys: Res<ButtonInput<KeyCode>>, mut mode: ResMut<NextState<Mode>>) {
    if keys.just_pressed(KeyCode::Escape) {
        mode.set(Mode::Playing);
    }
}

fn show_count(mut counts: MessageReader<Sleepers>, mut text: Single<&mut Text, With<CountText>>) {
    if let Some(c) = counts.read().last() {
        text.0 = if c.total > 1 {
            format!("Спят {} из {}. Ночь пройдёт, когда лягут все", c.asleep, c.total)
        } else {
            "Засыпаете…".into()
        };
    }
}
