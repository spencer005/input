use input_rs::events::InputEvent;
use std::mem::size_of;

fn main() {
    println!("Size of InputEvent: {} bytes", size_of::<InputEvent>());
    if size_of::<InputEvent>() > 48 {
        eprintln!("Warning: InputEvent is larger than 48 bytes ({}), consider boxing more variants", size_of::<InputEvent>());
    }
}
