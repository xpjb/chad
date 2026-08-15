#!/usr/bin/env sh
set -eu

cargo run --locked --example triangle -- --screenshot gallery/triangle.png
cargo run --locked --example halfpipe -- --screenshot gallery/halfpipe.png
cargo run --locked --example clock -- --screenshot gallery/clock.png
cargo run --locked --example sprite_batch -- --screenshot gallery/sprite_batch.png
cargo run --locked --example fractal_flight -- --screenshot gallery/fractal_flight.png
