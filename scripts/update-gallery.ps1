$ErrorActionPreference = "Stop"

function Invoke-Cargo {
    & cargo @args
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

Invoke-Cargo run --locked --example triangle -- --screenshot gallery/triangle.png
Invoke-Cargo run --locked --example halfpipe -- --screenshot gallery/halfpipe.png
Invoke-Cargo run --locked --example clock -- --screenshot gallery/clock.png
Invoke-Cargo run --locked --example sprite_batch -- --screenshot gallery/sprite_batch.png
Invoke-Cargo run --locked --example fractal_flight -- --screenshot gallery/fractal_flight.png
