//! Shared look of menus: panels, buttons and text fields.

use crate::sound::{PlaySound, Sfx};
use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    },
    prelude::*,
};

pub const ACCENT: Color = Color::srgb(0.95, 0.72, 0.32);
pub const PANEL: Color = Color::srgba(0.07, 0.08, 0.1, 0.86);
const BUTTON: Color = Color::srgba(1.0, 1.0, 1.0, 0.09);
const BUTTON_HOVER: Color = Color::srgba(1.0, 1.0, 1.0, 0.2);
const FIELD: Color = Color::srgba(0.0, 0.0, 0.0, 0.45);

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (type_into_fields, show_fields).chain());
    }
}

/// A full-screen layer that centres its children in a column.
pub fn screen(commands: &mut Commands, scope: impl Bundle, tint: Color) -> Entity {
    commands
        .spawn((
            scope,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: px(10),
                ..default()
            },
            BackgroundColor(tint),
            GlobalZIndex(5),
        ))
        .id()
}

/// A rounded column panel.
pub fn panel(commands: &mut Commands, parent: Entity, width: f32) -> Entity {
    commands
        .spawn((
            Node {
                width: px(width),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                padding: UiRect::all(px(18)),
                row_gap: px(8),
                border_radius: BorderRadius::all(px(12)),
                ..default()
            },
            BackgroundColor(PANEL),
            ChildOf(parent),
        ))
        .id()
}

pub fn title(commands: &mut Commands, parent: Entity, text: &str, size: f32) -> Entity {
    commands
        .spawn((
            Text::new(text),
            TextFont { font_size: FontSize::Px(size), ..default() },
            TextShadow { offset: Vec2::splat(1.5), color: Color::srgba(0.0, 0.0, 0.0, 0.6) },
            TextLayout::justify(Justify::Center),
            Node { margin: UiRect::bottom(px(size * 0.4)), align_self: AlignSelf::Center, ..default() },
            ChildOf(parent),
        ))
        .id()
}

pub fn label(commands: &mut Commands, parent: Entity, text: &str, size: f32, color: Color) -> Entity {
    commands
        .spawn((
            Text::new(text),
            TextFont { font_size: FontSize::Px(size), ..default() },
            TextColor(color),
            ChildOf(parent),
        ))
        .id()
}

/// A button; attach behaviour with `.observe(|_: On<PointerClick>, ...| ...)`.
pub fn button(commands: &mut Commands, parent: Entity, text: &str) -> Entity {
    commands
        .spawn((
            Node {
                min_height: px(44),
                padding: UiRect::axes(px(18), px(8)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(px(8)),
                ..default()
            },
            BackgroundColor(BUTTON),
            ChildOf(parent),
        ))
        .with_child((Text::new(text), TextFont { font_size: FontSize::Px(19.0), ..default() }, Pickable::IGNORE))
        .observe(|_: On<PointerClick>, mut sounds: MessageWriter<PlaySound>| {
            sounds.write(PlaySound { volume: 0.6, ..PlaySound::ui(Sfx::Click) });
        })
        .observe(|over: On<PointerOver>, mut colors: Query<&mut BackgroundColor>| {
            if let Ok(mut c) = colors.get_mut(over.event_target()) {
                c.0 = BUTTON_HOVER;
            }
        })
        .observe(|out: On<PointerOut>, mut colors: Query<&mut BackgroundColor>| {
            if let Ok(mut c) = colors.get_mut(out.event_target()) {
                c.0 = BUTTON;
            }
        })
        .id()
}

/// A horizontal row for laying out widgets side by side.
pub fn row(commands: &mut Commands, parent: Entity) -> Entity {
    commands
        .spawn((
            Node {
                column_gap: px(8),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                ..default()
            },
            ChildOf(parent),
        ))
        .id()
}

/// Single-line text input. Click to focus; typing goes to the focused field.
#[derive(Component)]
pub struct TextField {
    pub value: String,
    pub placeholder: String,
    pub max_len: usize,
    focused: bool,
}

pub fn text_field(commands: &mut Commands, parent: Entity, placeholder: &str, focused: bool) -> Entity {
    commands
        .spawn((
            TextField { value: String::new(), placeholder: placeholder.into(), max_len: 32, focused },
            Node {
                min_height: px(40),
                padding: UiRect::axes(px(12), px(8)),
                align_items: AlignItems::Center,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
            BackgroundColor(FIELD),
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.2)),
            ChildOf(parent),
        ))
        .with_child((Text::default(), TextFont { font_size: FontSize::Px(18.0), ..default() }, Pickable::IGNORE))
        .observe(|press: On<PointerPress>, mut fields: Query<(Entity, &mut TextField)>| {
            for (entity, mut field) in &mut fields {
                field.focused = entity == press.event_target();
            }
        })
        .id()
}

fn type_into_fields(mut keys: MessageReader<KeyboardInput>, mut fields: Query<&mut TextField>) {
    let Some(mut field) = fields.iter_mut().find(|f| f.focused) else {
        keys.clear();
        return;
    };
    for key in keys.read().filter(|k| k.state == ButtonState::Pressed) {
        match &key.logical_key {
            Key::Backspace => {
                field.value.pop();
            }
            Key::Character(_) | Key::Space => {
                if let Some(text) = &key.text {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        if field.value.chars().count() < field.max_len {
                            field.value.push(c);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn show_fields(
    time: Res<Time<Real>>,
    fields: Query<(&TextField, &Children, &mut BorderColor)>,
    mut texts: Query<(&mut Text, &mut TextColor)>,
) {
    let caret = time.elapsed_secs().fract() < 0.5;
    for (field, children, mut border) in fields {
        *border = BorderColor::all(if field.focused { ACCENT } else { Color::srgba(1.0, 1.0, 1.0, 0.2) });
        let Ok((mut text, mut color)) = texts.get_mut(children[0]) else { continue };
        let shown = if field.value.is_empty() && !field.focused {
            color.0 = Color::srgba(1.0, 1.0, 1.0, 0.4);
            field.placeholder.clone()
        } else {
            color.0 = Color::WHITE;
            format!("{}{}", field.value, if field.focused && caret { "|" } else { " " })
        };
        if text.0 != shown {
            text.0 = shown;
        }
    }
}
