# incomplete libinput alternative

Used in my custom compositor

vibe disclaimer: this is in an incomplete state, not completely audited, only known to work for my personal use case

although it does work perfectly fine for that case. Not fully audited.

Things different to libinput:
- Avoids compounding inaccuracy bias towards a fixed axis (causes multiple pixel inaccuracy in just a few seconds)
- Doesn't force debounce on even for devices that don't need it (config is absolutely not finished)
- Specifically optimizes for the raw input compositor case, bypassing all extra transformations like non-`flat` accel curves
- would accept more features/use cases, be less conservative
- abstracted over evdev (not linux specific)
- dpi config exposed -- if your mouse dpi is set to something crazy like 10000 you can normalize it to a normal speed even for raw input cases (games) which solves you having to use insane sensitivity values like 0.01 which sometimes aren't available or easy to set. eventually the goal would be integrating with each mouse's software if available to get the current dpi.
