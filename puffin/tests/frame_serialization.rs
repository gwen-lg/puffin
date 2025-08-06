use std::{sync::Arc, thread::sleep, time::Duration};

use puffin::{FrameData, GlobalProfiler};

fn process_1() {
    puffin::profile_function!();
    sub_process_1_1();
    (0..2).for_each(|_| sub_process_1_2());
}

fn sub_process_1_1() {
    puffin::profile_function!();
    sleep(Duration::from_millis(1));
}

fn sub_process_1_2() {
    puffin::profile_function!();
    sleep(Duration::from_micros(2));
}

#[test]
fn single_frame() {
    fn profiler_sink(frame_data: Arc<FrameData>) {
        let frame_meta = frame_data.meta();
        assert_eq!(frame_meta.frame_index, 0);
        assert_eq!(frame_meta.num_scopes, 4);
    }

    // Init profiler sink and enable capture
    let sink_id = GlobalProfiler::lock().add_sink(Box::new(profiler_sink));
    puffin::set_scopes_on(true);

    // Run process
    process_1();

    // End frame, and uninit profiler
    puffin::GlobalProfiler::lock().new_frame();
    GlobalProfiler::lock().remove_sink(sink_id);
}
