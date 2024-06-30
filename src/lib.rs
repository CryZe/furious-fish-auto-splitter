#![no_std]

use asr::{
    future::next_tick,
    itoa,
    settings::Gui,
    string::ArrayString,
    timer::{self, TimerState},
    watcher::Watcher,
    PointerSize, Process,
};
use bytemuck::{Pod, Zeroable};

asr::async_main!(stable);
asr::panic_handler!();

#[derive(Copy, Clone, Gui)]
enum When {
    /// Every 10m
    Meters10,
    /// Every 25m
    Meters25,
    /// Every 50m
    Meters50,
    /// Every 100m
    Meters100,
    #[default]
    /// Only at the end
    End,
}

impl When {
    fn to_chunk(self, meters: f32) -> i32 {
        let factor = match self {
            When::Meters10 => 1.0 / 10.0,
            When::Meters25 => 1.0 / 25.0,
            When::Meters50 => 1.0 / 50.0,
            When::Meters100 => 1.0 / 100.0,
            When::End => 0.0,
        };
        (meters * factor) as i32
    }
}

#[derive(Gui)]
struct Settings {
    /// When to split:
    ///
    /// You can split at various heights or only at the end. You need to create
    /// splits for each multiple that you choose and one for the end. The end is
    /// at 239m.
    when: When,
}

#[derive(Copy, Clone, Pod, Zeroable)]
#[repr(C)]
struct Position {
    x: f32,
    y: f32,
}

impl Position {
    fn meters(self) -> f32 {
        (300.0 - self.y) * (1.0 / 200.0)
    }
}

async fn main() {
    let mut settings = Settings::register();

    loop {
        let process = Process::wait_attach("Furious Fish.exe").await;
        process
            .until_closes(async {
                let (module, _) = process.wait_module_range("Furious Fish.exe").await;
                let mut max_chunk = 0;
                let mut max_height = 0.0;
                let mut position = Watcher::new();
                loop {
                    if let Some(position) = position.update(
                        process
                            .read_pointer_path::<Position>(
                                module,
                                PointerSize::Bit64,
                                &[0x0424BE40, 0x288, 0x0, 0x460],
                            )
                            .ok(),
                    ) {
                        let meters = position.meters();

                        match timer::state() {
                            TimerState::NotRunning => {
                                if position.check(|h| h.y != -25.0) {
                                    timer::start();
                                }
                                max_chunk = 0;
                                max_height = 0.0;
                            }
                            TimerState::Running => {
                                if meters > max_height {
                                    max_height = meters;
                                }

                                settings.update();

                                let chunk = settings.when.to_chunk(meters);

                                if (chunk > max_chunk && position.y > -47620.0)
                                    || (position.y < -47620.0
                                        && position.x < -85.0
                                        && position.y > -47626.0)
                                {
                                    max_chunk = chunk;
                                    timer::split();
                                }
                            }
                            _ => {}
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

                    next_tick().await;
                }
            })
            .await;

        if timer::state() != TimerState::NotRunning {
            timer::reset();
        }
    }
}
