# Why the Rust and Tauri scaffold failed

Update, 12 November 2026: The maximised tab delay was repaired by disabling CSS shadows. This postmortem records the earlier state. See [the performance fix](performance_fix_2026-11-12.md) for measurements and current status.

Date: 12 November 2026  
Decision: Stop treating this scaffold as a reusable desktop application template.

## Verdict

This project meets many written feature requirements, but it fails the requirement that matters most in use: the interface must respond promptly. Kim reports slow scrolling, tab changes, buttons and drag and drop. Maximised, a tab change can take several seconds. A desktop template that behaves that way is unsuitable for copying into future projects. The 4.3 MB binary, passing unit tests and finished documentation do not compensate for an interface that feels unusable.

This is a verdict on **this scaffold**, not on Rust or every Tauri application. The responsive water tracker on the same machine is evidence that a small Tauri app can work well. The precise bottleneck in this scaffold remains unproved. Abandoning it as a template is a product decision based on its observed behaviour, not a claim that the root cause has been isolated.

## What went wrong in the investigation

1. **Claude tested a different launch path from Kim's.** Claude started the finished binary directly, detached from a terminal, with standard output and errors sent to `/dev/null`. Kim used `./run.sh`, which built if needed and then ran the app in the foreground attached to her terminal. Those were materially different runtime conditions. Claude described its own runs as responsive without establishing that they represented Kim's normal launch. Kim noticed the mismatch while watching Claude work.
2. **The mismatch was discovered late.** After Kim identified it, `run.sh` was changed to launch the app detached. Kim reported a partial improvement. This confirms that the comparison had been inadequate. It does **not** prove why detaching helped, or explain the severe delay that remains when maximised. The build step itself finishes before the app starts, so it should not be presented as a proven runtime cause.
3. **The measurements missed the failure case.** Claude measured CPU ticks during scripted mouse actions at smaller window sizes. Kim's complaint concerned what she felt with a real mouse, especially in a maximised window. The handoff states that maximised latency was never measured. A scripted tab switch using 0.13 to 0.2 seconds of web-process CPU does not refute a several-second visible delay in the unmeasured condition.
4. **The working app was mistaken for a successful template.** The project acquired themes, a tray icon, configuration persistence, sortable cards, accordions, tables and polished styling while the performance complaint remained open. Eleven Rust unit tests verify config and other logic, but they say nothing about paint time, input latency or scrolling. The initial complaint was deferred instead of being used as a gate before more interface work.
5. **The only useful comparison was not equivalent.** The water tracker has a fixed 360 x 540 window and uses an opaque Tauri window with picom clipping its corners. This scaffold has a transparent, resizable window that can cover about 1920 x 1028 pixels. The water tracker was never tested maximised. Its responsiveness therefore cannot establish that the large scaffold window should behave the same way.

These failures made the progress reports misleading in effect: they conveyed confidence in responsiveness that the tests did not support. The record does not establish deliberate deception or sabotage, and this document makes no such allegation.

## What is known about the slowdown

The normal 640 x 480 window covers about 307,000 pixels. Maximised at 1920 x 1028, it covers about 1,974,000 pixels, 6.4 times as much. The app draws a rounded, clipped frame and layered content in a transparent window. Earlier measurements show substantial WebKit web-process activity during resizing. Together these observations make rendering cost at large window sizes the leading hypothesis. They do not prove that each interaction repaints the whole window or identify a single CSS rule as the cause.

The tabs use Bootstrap show/hide behaviour. A tab click does not call Rust or save the config file. `ui/js/window.js` does call Tauri's `isMaximized()` on every resize event, which can add work during resizing or maximising; it does not explain slow tab clicks after the resize ends. Experiments at smaller sizes did not isolate transparency, outer-frame shadows, the gradient or Aero Blue's blur as the cause. Component shadows, large-area clipping, input differences and combinations of effects remain untested in the maximised case.

## Why this template should be retired

A reusable scaffold is meant to reduce risk and work for future applications. This one carries an unresolved responsiveness defect into every project that copies it. Its most important performance case was never verified, and the final launcher change only partly improved it. Further work could diagnose and repair it, but there is no demonstrated fix to inherit today. Calling the template complete would transfer the same problem to the next app.

Stop using this scaffold as the starting point for new projects. Preserve the source and the investigation records for reference. Do not delete it or rebuild a collection of applications from it on the assumption that a single setting will fix the issue. A future attempt should first prove that a minimal, maximised window responds promptly under Kim's actual launch and input conditions, then add styling and features one step at a time.

## Evidence and limits

- `AGENTS.md`: dated engineering log, launch change, user reports and known issue.
- `docs/project_failure_handoff_guide.md`: symptoms, measurements, test limitations and comparison with the water tracker.
- `docs/performance_investigation_report.md`: independent code review and explicit uncertainty about the root cause.
- `run.sh`: current detached launcher. The original foreground behaviour is documented in the engineering log and handoff.

No new maximised benchmark was run for this postmortem. The exact rendering or input bottleneck remains unresolved. The failure decision rests on the observed user experience and the absence of a verified fix.
