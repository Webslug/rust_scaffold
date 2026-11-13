One point four gigs of deps for a four meg binary is peak Rust. That's the price of static linking and monomorphization — every generic gets stamped out into its own copy, so the linker throws away a ton but only after chewing through all of it. Mold as your linker can cut link times by half if you haven't tried it.

---

Mold is a drop-in replacement linker. Instead of using the default GNU linker or LLD that Rust ships with, you point your build at mold and it does the same job — merging all those object files into one binary — but way faster, mostly because it parallelizes the work across every CPU core instead of grinding through it single-threaded like the classics do. You set it with an environment variable, RUSTFLAGS equals dash C link-arg equals dash dashfuse-ld equals mold, or you can bake it into your .cargo config so it's permanent. Zero code changes, just a faster link step, which for a Tauri app is where most of your compile time goes after the first build.
