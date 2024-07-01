#![no_std]

use asr::{
    future::next_tick,
    itoa,
    settings::Gui,
    string::ArrayString,
    timer::{self, TimerState},
    watcher::Watcher,
    Process,
};

mod godot;

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

                let (scene_tree, root_node) = asr::future::retry(|| {
                    let scene_tree = godot::SceneTree::get(&process, module).ok()?;
                    let root_node = scene_tree.get_root(&process).ok()?;
                    Some((scene_tree, root_node))
                })
                .await;

                asr::print_message("Found root node");

                let (player_node, start_frame) = asr::future::retry(|| {
                    // FIXME: The last scene is the "most active one". Michael
                    // apparently forgot to remove the previous scenes when
                    // navigating the title and shop. If they don't fix it, we
                    // should at least iterate backwards.
                    let game_node = root_node
                        .children()
                        .iter(&process)
                        .last()?
                        .1
                        .deref(&process)
                        .ok()?;

                    // FIXME: Do a proper hash map lookup.

                    let player_node = game_node
                        .children()
                        .iter(&process)
                        .find_map(|(name, node)| {
                            if name
                                .deref(&process)
                                .ok()?
                                .read::<6>(&process)
                                .ok()?
                                .matches_str("Player")
                            {
                                node.deref(&process).ok()
                            } else {
                                None
                            }
                        })?
                        .cast::<godot::Node2D>();

                    let start_frame = scene_tree.get_current_frame(&process).ok()?;

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

                    let Ok(frame) = scene_tree.get_current_frame(&process) else {
                        continue;
                    };

                    let meters = to_meters(y.current);

                    if timer::state() == TimerState::Running {
                        if meters > max_height {
                            max_height = meters;
                        }

                        timer::set_game_time(asr::time_util::frame_count::<60>(
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

fn should_split(y: f32, settings: &mut Settings, max_chunk: &mut i32) -> bool {
    if y <= -112578.67 {
        return true;
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
