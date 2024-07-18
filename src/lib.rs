#![no_std]

use asr::{
    future::{next_tick, retry},
    game_engine::godot::{CSharpScriptInstance, Node2D, SceneTree},
    itoa, set_tick_rate,
    settings::Gui,
    string::ArrayString,
    time::Duration,
    timer::{self, TimerState},
    Process,
};
use bytemuck::{Pod, Zeroable};

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
    /// at 1258m. The last split before the end is at 1150m or below depending
    /// on the setting.
    when: When,
}

#[derive(Copy, Clone, Zeroable, Pod)]
#[repr(C)]
struct Stats {
    total_distance_fallen: f32,
    total_jumps: i32,
    total_time: f32,
}

fn to_meters(y: f32) -> f32 {
    y * TO_METERS as f32
}

impl Stats {
    fn calculate_sexiness(&self) -> &'static str {
        // TODO: Exclusive ranges in Rust 1.80 (2024-07-25)
        match (self.total_distance_fallen * 0.01) as i32 {
            ..=99 => "S+",
            100..=299 => "S",
            300..=499 => "S-",
            500..=799 => "A+",
            800..=1199 => "A",
            1200..=1799 => "A-",
            1800..=2499 => "B+",
            2500..=3499 => "B",
            3500..=4999 => "B-",
            5000..=6999 => "C+",
            7000..=9499 => "C",
            9500..=13999 => "C-",
            14000..=19999 => "D+",
            20000..=29999 => "D",
            30000..=39999 => "D-",
            40000..=49999 => "F+",
            50000.. => "F",
        }
    }
}

#[derive(Default)]
struct Run {
    max_chunk: i32,
    max_height: f32,
    previous_time: f32,
}

async fn main() {
    let mut settings = Settings::register();

    let mut run = Run::default();

    loop {
        set_tick_rate(1.0);

        let process = Process::wait_attach("Furious Fish.exe").await;
        process
            .until_closes(async {
                set_tick_rate(120.0);

                let module = retry(|| process.get_module_address("Furious Fish.exe")).await;

                let scene_tree = SceneTree::wait_locate(&process, module).await;
                let root_node = scene_tree.wait_get_root(&process).await;

                asr::print_message("Found root");

                'look_for_player: loop {
                    let (player_node, game_node, script) = retry(|| {
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

                        let script = player_node
                            .get_script_instance(&process)
                            .ok()??
                            .unchecked_cast::<CSharpScriptInstance>()
                            .get_gc_handle(&process)
                            .ok()?;

                        Some((player_node, game_node, script))
                    })
                    .await;

                    if timer::state() == TimerState::NotRunning {
                        timer::start();
                        run = Run::default();
                    } else {
                        timer::resume_game_time();
                        // Delay on restarting the game, as the save file may
                        // not have been loaded yet.
                        for _ in 0..60 {
                            next_tick().await;
                        }
                    }
                    timer::pause_game_time();

                    asr::print_message("Found player");

                    loop {
                        next_tick().await;

                        let Ok(position @ [_, y]) = player_node.get_position(&process) else {
                            continue;
                        };

                        let Ok(instance_data) = script.get_instance_data(&process) else {
                            continue;
                        };

                        // player.totalDistanceFallen (use .NET Info in Cheat Engine to
                        // find the offset)
                        let Ok(stats) =
                            instance_data.read_at_byte_offset::<Stats, _>(0x1c4, &process)
                        else {
                            continue;
                        };

                        if run.previous_time > stats.total_time {
                            timer::reset();
                            timer::start();
                            timer::pause_game_time();
                            run = Run::default();
                        }
                        run.previous_time = stats.total_time;

                        let meters = to_meters(y);

                        if meters > run.max_height {
                            run.max_height = meters;
                        }

                        timer::set_game_time(Duration::saturating_seconds_f32(stats.total_time));

                        if let Some(is_at_end) =
                            should_split(position, &mut settings, &mut run.max_chunk)
                        {
                            timer::split();

                            if is_at_end {
                                // Once we reach the end, just wait for the game scene
                                // to get unloaded.
                                loop {
                                    next_tick().await;

                                    let Some((_, last_node)) =
                                        root_node.get_children().iter_back(&process).next()
                                    else {
                                        continue;
                                    };

                                    let Ok(last_node) = last_node.deref(&process) else {
                                        continue;
                                    };

                                    if last_node.addr() != game_node.addr() {
                                        asr::print_message("Back to title");
                                        continue 'look_for_player;
                                    }
                                }
                            }
                        }

                        let mut buf = ArrayString::<16>::new();
                        let mut itoa_buf = itoa::Buffer::new();
                        let _ = buf.try_push_str(itoa_buf.format(meters as i32));
                        let _ = buf.try_push('m');

                        timer::set_variable("Height", &buf);

                        buf.clear();
                        let _ = buf.try_push_str(itoa_buf.format(run.max_height as i32));
                        let _ = buf.try_push('m');

                        timer::set_variable("Max Height", &buf);

                        timer::set_variable("Jumps", itoa_buf.format(stats.total_jumps));

                        timer::set_variable("Sexiness", stats.calculate_sexiness());
                    }
                }
            })
            .await;
    }
}

#[allow(clippy::inconsistent_digit_grouping)]
fn should_split([x, y]: [f32; 2], settings: &mut Settings, max_chunk: &mut i32) -> Option<bool> {
    if y <= -1258_37.3 && (-390.5632..=121.44321).contains(&x) {
        return Some(true);
    }

    if y < -1160_00.0 {
        // This is to make sure that 1150m is the last split before the end.
        return None;
    }

    settings.update();

    let chunk = settings.when.to_chunk(y);

    if chunk > *max_chunk {
        *max_chunk = chunk;
        Some(false)
    } else {
        None
    }
}
