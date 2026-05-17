use input_rs::processing::{FilterKind};
use input_rs::processing::touchpad_motion::TouchpadMotionFilter;
use input_rs::processing::mouse::MouseFilter;
use input_rs::processing::debounce::ButtonDebounceFilter;
use std::mem::size_of;

fn main() {
    println!("Size of TouchpadMotionFilter: {} bytes", size_of::<TouchpadMotionFilter>());
    println!("Size of MouseFilter: {} bytes", size_of::<MouseFilter>());
    println!("Size of ButtonDebounceFilter: {} bytes", size_of::<ButtonDebounceFilter>());
    println!("Size of FilterKind: {} bytes", size_of::<FilterKind>());
}
