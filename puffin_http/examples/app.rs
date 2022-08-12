use rand::random;
use std::{thread, time};

#[derive(Clone, Copy)]
enum LoopBehavior {
	Unlimited,
	Limited(u32),
}

fn main() {
	let server_addr = format!("0.0.0.0:{}", puffin_http::DEFAULT_PORT);
	eprintln!("Serving demo profile data on {}", server_addr);
	let puffin_server = puffin_http::Server::new(&server_addr).unwrap();

	puffin::set_scopes_on(true); // need this to enable capture

    // wait client(s) connection
    //while puffin_server.num_clients() == 0 {}

	let loop_behavior = LoopBehavior::Limited(35);

	let mut loop_count = 0;
	while continue_loop(loop_behavior, loop_count) {
        let data = format!("Main loop num {}", loop_count);
		puffin::profile_scope!("main_loop", data);
		puffin::GlobalProfiler::lock().new_frame();
		if false { // loop_count == 0
			// Big sleep to simulate loading
			puffin::profile_scope!("loading");
			let sleep_duration = time::Duration::from_secs(10);
			thread::sleep(sleep_duration);

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
