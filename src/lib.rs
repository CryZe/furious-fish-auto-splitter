#![no_std]

use asr::{
    future::next_tick,
    game_engine::godot::{Node2D, SceneTree},
    itoa,
    settings::Gui,
    string::ArrayString,
    time_util,
    timer::{self, TimerState},
    watcher::Watcher,
    Process,
};

asr::async_main!(stable);
asr::panic_handler!();

#[derive(Copy, Clone, Gui)]
enum When {
    /// Every 25m
    Meters25,
    /// Every 50m
    Meters50,
    /// Every 100m
    Meters100,
    /// Every 250m
    Meters250,
    #[default]
    /// Only at the end
    End,
}

const TO_METERS: f64 = -1.0 / 100.0;

impl When {
    fn to_chunk(self, y: f32) -> i32 {
        let factor = match self {
            When::Meters25 => (TO_METERS / 25.0) as f32,
            When::Meters50 => (TO_METERS / 50.0) as f32,
            When::Meters100 => (TO_METERS / 100.0) as f32,
            When::Meters250 => (TO_METERS / 250.0) as f32,
            When::End => 0.0,
        };
        (y * factor) as i32
    }
}

#[derive(Gui)]
struct Settings {
    /// When to split:
    ///
    /// You can split at various heights or only at the end. You need to create
    /// splits for each multiple that you choose and one for the end. The end is
    /// at 1125m.
    when: When,
}

fn to_meters(y: f32) -> f32 {
    y * TO_METERS as f32
}

async fn main() {
    let mut settings = Settings::register();

    loop {
        let process = Process::wait_attach("Furious Fish.exe").await;
        process
            .until_closes(async {
                let (module, _) = process.wait_module_range("Furious Fish.exe").await;

                let scene_tree = SceneTree::wait_locate(&process, module).await;
                let root_node = scene_tree.wait_get_root(&process).await;

                asr::print_message("Found root node");

                let (player_node, start_frame) = asr::future::retry(|| {
                    // FIXME: The last scene is the "most active one". Michael
                    // apparently forgot to remove the previous scenes when
                    // navigating the title and shop.
                    let game_node = root_node
                        .get_children()
                        .iter_back(&process)
                        .next()?
                        .1
                        .deref(&process)
                        .ok()?;

                    let player_node = game_node
                        .find_child(b"Player", &process)
                        .ok()??
                        .unchecked_cast::<Node2D>();

                    let start_frame = scene_tree.get_frame(&process).ok()?;

                    Some((player_node, start_frame))
                })
                .await;

                timer::start();
                timer::pause_game_time();

                let mut max_chunk = 0;
                let mut max_height = 0.0;
                let mut height = Watcher::new();

                loop {
                    next_tick().await;

                    let Some(y) =
                        height.update(player_node.get_position(&process).ok().map(|[_, y]| y))
                    else {
                        continue;
                    };

                    let Ok(frame) = scene_tree.get_frame(&process) else {
                        continue;
                    };

                    let meters = to_meters(y.current);

                    if timer::state() == TimerState::Running {
                        if meters > max_height {
                            max_height = meters;
                        }

                        timer::set_game_time(time_util::frame_count::<60>(
                            (frame - start_frame) as u64,
                        ));

                        if should_split(y.current, &mut settings, &mut max_chunk) {
                            timer::split();
                        }
                    }

                    let mut buf = ArrayString::<16>::new();
                    let mut itoa_buf = itoa::Buffer::new();
                    let _ = buf.try_push_str(itoa_buf.format(meters as i32));
                    let _ = buf.try_push('m');

                    timer::set_variable("Height", &buf);

                    buf.clear();
                    let _ = buf.try_push_str(itoa_buf.format(max_height as i32));
                    let _ = buf.try_push('m');

                    timer::set_variable("Max Height", &buf);
                }
            })
            .await;

        if timer::state() != TimerState::NotRunning {
            timer::reset();
        }
    }
}

#[allow(clippy::inconsistent_digit_grouping)]
fn should_split(y: f32, settings: &mut Settings, max_chunk: &mut i32) -> bool {
    if y <= -1125_78.67 {
        return true;
    }

    if y < -1120_00.0 {
        // This is to make sure that at 1125m we only split the very final
        // split, not any multiple.
        return false;
    }

    settings.update();

    let chunk = settings.when.to_chunk(y);

    if chunk > *max_chunk {
        *max_chunk = chunk;
        true
    } else {
        false
    }
}
