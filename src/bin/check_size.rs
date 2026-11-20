use {
    input_rs::{
        events::InputEvent,
        processing::{FilterResult, Step},
    },
    std::mem::size_of,
};

fn main() {
    println!("Size of InputEvent: {} bytes", size_of::<InputEvent>());
    println!("Size of FilterResult: {} bytes", size_of::<FilterResult>());
    println!("Size of Step: {} bytes", size_of::<Step>());
    if size_of::<InputEvent>() > 48 {
        eprintln!(
            "Warning: InputEvent is larger than 48 bytes ({}), consider boxing more variants",
            size_of::<InputEvent>()
        );
    }
}
