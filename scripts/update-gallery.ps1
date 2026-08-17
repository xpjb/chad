$ErrorActionPreference = "Stop"

function Update-Screenshot($Example, $Path) {
    & cargo run --locked --example $Example -- --screenshot $Path
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

Update-Screenshot triangle gallery/triangle.png
Update-Screenshot halfpipe gallery/halfpipe.png
Update-Screenshot clock gallery/clock.png
Update-Screenshot sprite_batch gallery/sprite_batch.png
Update-Screenshot fractal_flight gallery/fractal_flight.png
