# Install the Rust toolchain from the devbox's mise config, for building and testing the
# Sherpa client on Windows. Safe to run again: mise skips what is installed.
$ErrorActionPreference = 'Stop'

mise install rust
if ($LASTEXITCODE -ne 0) { throw "mise install rust failed with exit code $LASTEXITCODE" }
mise exec rust -- rustc --version
mise exec rust -- cargo --version
