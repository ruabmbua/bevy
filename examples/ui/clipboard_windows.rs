//! Copy and paste text through the system clipboard from two windows.
//!
//! Press `C` in a window to copy a line naming that window, `D` to do so three seconds later,
//! and `V` to paste: results are shown in the window and logged. Every three seconds, the
//! clipboard is also read without a key press. `D` and these reads show what the app can do
//! while none of its windows has keyboard focus.
//!
//! Run with `--features system_clipboard`.

use bevy::{
    camera::RenderTarget,
    input::keyboard::{Key, KeyboardInput},
    prelude::*,
    window::{ExitCondition, PrimaryWindow, WindowRef},
};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            // Either window may be closed first.
            exit_condition: ExitCondition::OnAllClosed,
            ..default()
        }))
        .add_systems(Startup, setup)
        .add_systems(Update, (copy_paste, poll))
        .run();
}

#[derive(Component)]
struct Status(Entity);

fn setup(mut commands: Commands, primary: Single<Entity, With<PrimaryWindow>>) {
    let second = commands
        .spawn(Window {
            title: "Second window".to_owned(),
            ..default()
        })
        .id();
    for window in [*primary, second] {
        let camera = commands
            .spawn((Camera2d, RenderTarget::Window(WindowRef::Entity(window))))
            .id();
        commands.spawn((
            Text::new("C: copy, D: copy in 3 s, V: paste"),
            Status(window),
            UiTargetCamera(camera),
        ));
    }
}

fn copy_paste(
    mut keys: MessageReader<KeyboardInput>,
    mut clipboard: ResMut<Clipboard>,
    windows: Query<&Window>,
    mut statuses: Query<(&Status, &mut Text)>,
    mut delayed: Local<Option<(Timer, String)>>,
    time: Res<Time>,
) {
    if let Some((timer, text)) = delayed.as_mut()
        && timer.tick(time.delta()).just_finished()
    {
        info!("delayed copy: {:?}", clipboard.set_text(text.clone()));
        *delayed = None;
    }
    for key in keys.read().filter(|key| key.state.is_pressed()) {
        let title = windows.get(key.window).map_or("?", |w| w.title.as_str());
        let Key::Character(character) = &key.logical_key else {
            continue;
        };
        let status = match character.as_str() {
            "c" => {
                let result = clipboard.set_text(format!("copied from {title}"));
                info!("copy in {title}: {result:?}");
                format!("copy: {result:?}")
            }
            "d" => {
                let text = format!("copied from {title}, delayed");
                *delayed = Some((Timer::from_seconds(3.0, TimerMode::Once), text));
                "copying in 3 s".to_owned()
            }
            "v" => {
                let result = clipboard.fetch_text().poll_result();
                info!("paste in {title}: {result:?}");
                format!("paste: {result:?}")
            }
            _ => continue,
        };
        for (_, mut text) in statuses.iter_mut().filter(|(s, _)| s.0 == key.window) {
            text.0 = status.clone();
        }
    }
}

fn poll(time: Res<Time>, mut timer: Local<Option<Timer>>, mut clipboard: ResMut<Clipboard>) {
    let timer = timer.get_or_insert_with(|| Timer::from_seconds(3.0, TimerMode::Repeating));
    if timer.tick(time.delta()).just_finished() {
        info!("poll: {:?}", clipboard.fetch_text().poll_result());
    }
}
