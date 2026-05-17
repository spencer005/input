use std::path::PathBuf;
use std::fs;
use std::io;
use std::mem::MaybeUninit;
use rustix::fs::inotify;
use rustix::fd::OwnedFd;
use super::{BusType, DeviceDiscovery, DiscoveryEvent};

/// Read the device name from sysfs. Returns "Unknown" on failure.
fn read_device_name(event_dev: &str) -> String {
    let path = format!("/sys/class/input/{event_dev}/device/name");
    fs::read_to_string(&path)
        .map(|s| s.trim_end().to_string())
        .unwrap_or_else(|_| "Unknown".to_string())
}

/// Read a hex value from a sysfs file. Returns 0 on failure.
fn read_sysfs_hex(path: &str) -> u16 {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| u16::from_str_radix(s.trim(), 16).ok())
        .unwrap_or(0)
}

/// Determine bus type from the kernel bustype value.
fn bustype_from_kernel(v: u16) -> BusType {
    match v {
        0x03 => BusType::Usb,
        0x05 => BusType::Bluetooth,
        0x10 => BusType::I2c,
        0x11 => BusType::Ps2,
        _ => BusType::Unknown,
    }
}

pub struct LinuxDiscovery {
    inotify_fd: OwnedFd,
}

impl LinuxDiscovery {
    pub fn new() -> std::io::Result<Self> {
        let fd = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)
            .map_err(|e| io::Error::from(e))?;

        let _wd = inotify::add_watch(&fd, "/dev/input", inotify::WatchFlags::CREATE | inotify::WatchFlags::DELETE)
            .map_err(|e| io::Error::from(e))?;

        Ok(Self {
            inotify_fd: fd,
        })
    }

    pub fn initial_scan() -> Vec<DiscoveryEvent> {
        let mut events = Vec::new();
        if let Ok(entries) = fs::read_dir("/dev/input") {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("event") {
                    let event_dev = name_str.as_ref();
                    events.push(DiscoveryEvent::Added {
                        path: entry.path(),
                        name: read_device_name(event_dev),
                    });
                }
            }
        }
        events
    }
}

impl DeviceDiscovery for LinuxDiscovery {
    fn poll(&mut self) -> Vec<DiscoveryEvent> {
        let mut events = Vec::new();
        let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];

        let mut iter = inotify::Reader::new(&self.inotify_fd, &mut buffer);
        while let Ok(entry) = iter.next() {
            if let Some(name_bytes) = entry.file_name() {
                let name_str = name_bytes.to_string_lossy();

                if name_str.starts_with("event") {
                    let path = PathBuf::from("/dev/input").join(&*name_str);
                    let mask = entry.events();
                    if mask.contains(inotify::ReadFlags::CREATE) {
                        let event_dev = name_str.as_ref();
                        events.push(DiscoveryEvent::Added {
                            path,
                            name: read_device_name(event_dev),
                        });
                    } else if mask.contains(inotify::ReadFlags::DELETE) {
                        events.push(DiscoveryEvent::Removed {
                            path,
                        });
                    }
                }
            }
        }

        events
    }
}

/// Read device identity (vendor, product, bus) from sysfs for an event device.
/// Returns (vendor_id, product_id, bus_type). Values are 0/Unknown on failure.
pub fn read_device_identity(event_dev: &str) -> (u16, u16, BusType) {
    let base = format!("/sys/class/input/{event_dev}/device/id");
    let vendor = read_sysfs_hex(&format!("{base}/vendor"));
    let product = read_sysfs_hex(&format!("{base}/product"));
    let bustype = read_sysfs_hex(&format!("{base}/bustype"));
    (vendor, product, bustype_from_kernel(bustype))
}
