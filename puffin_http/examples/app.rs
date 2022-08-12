use rand::random;
use std::{thread, time};

#[derive(Clone, Copy)]
enum LoopBehavior {
    Unlimited,
    Limited(u32),
}

#[derive(PartialEq, Clone, Copy)]
enum LoadingBehavior {
    None,
    PreLoop,
    FirstLoop,
    //Parrallel, TODO: add thread loading management
}

struct AppBehavior {
    pub loading: LoadingBehavior,
    pub loop_behavior: LoopBehavior,
}

impl Default for AppBehavior {
    fn default() -> Self {
        Self {
            loading: LoadingBehavior::None,
            loop_behavior: LoopBehavior::Limited(35),
        }
    }
}

fn main() {
    let server_addr = format!("0.0.0.0:{}", puffin_http::DEFAULT_PORT);
    eprintln!("Serving demo profile data on {}", server_addr);
    let _puffin_server = puffin_http::Server::new(&server_addr).unwrap();

    puffin::set_scopes_on(true); // need this to enable capture

    let app_behavior = AppBehavior::default();

    simulate_loading(app_behavior.loading, LoadingBehavior::PreLoop);

    let mut loop_count = 0;
    while continue_loop(app_behavior.loop_behavior, loop_count) {
        puffin::profile_scope!("main_loop", format!("main_loop {}", loop_count));
        puffin::GlobalProfiler::lock().new_frame();
        if loop_count == 0 {
            // Big sleep to simulate loading
            simulate_loading(app_behavior.loading, LoadingBehavior::FirstLoop);
        }
        loop_count += 1;

        let loop_duration = compute_loop_duration();
        {
            puffin::profile_scope!("sleep");
            let sleep_duration = time::Duration::from_millis(loop_duration);
            thread::sleep(sleep_duration);
        }
        println!("loop {} duration {}ms", loop_count, loop_duration);
    }
}

fn simulate_loading(behavior: LoadingBehavior, step: LoadingBehavior) {
    if behavior == step {
        puffin::profile_scope!("loading");
        let sleep_duration = time::Duration::from_secs(5);
        thread::sleep(sleep_duration);
    }
}

fn compute_loop_duration() -> u64 {
    const MIN_DURATION: u64 = 10;
    const MAX_DURATION: u64 = 55;
    let rand_value: u64 = random::<u64>() % (MAX_DURATION - MIN_DURATION);
    MIN_DURATION + rand_value
}

fn continue_loop(loop_behavior: LoopBehavior, loop_count: u32) -> bool {
    let continue_loop = match loop_behavior {
        LoopBehavior::Unlimited => true,
        LoopBehavior::Limited(limit) => loop_count < limit,
    };
    continue_loop
}
