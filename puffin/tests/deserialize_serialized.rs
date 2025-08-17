mod common;

use std::{io::Seek, thread, time::Duration};

use memfile::MemFile;

fn run_write(file: MemFile) {
    let _frame_writer = common::init_frames_writer(file);

    //println!("set_scopes_on(true)");
    puffin::set_scopes_on(true);
    // need this to enable capture

    common::example_run();

    //println!("set_scopes_on(false)");
    puffin::set_scopes_on(false);
    puffin::GlobalProfiler::lock().new_frame();
    //Force to get last frame
}

fn run_read(mut file: MemFile) {
    file.rewind().unwrap();
    let _ = puffin::FrameView::read(&mut file).expect("read :");
}

#[test]
fn deserialize_serialized() {
    let file = MemFile::create_default("deserialize_serialized.puffin").unwrap();
    run_write(file.try_clone().unwrap());
    thread::sleep(Duration::from_secs(1));
    run_read(file);
}
