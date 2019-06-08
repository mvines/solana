#!/usr/bin/env bash

[[ -n $APPVEYOR ]] || {
  echo APPVEYOR not defined
  exit 1
}

source ci/env.sh
source ci/rust-version.sh

set -x
#appveyor DownloadFile https://win.rustup.rs/ -FileName rustup-init.exe
#./rustup-init -yv --default-toolchain $rust_stable --default-host x86_64-pc-windows-msvc
#export PATH="$PATH:$USERPROFILE/.cargo/bin"
#rustc -vV
#cargo -vV
ci/publish-tarball.sh
