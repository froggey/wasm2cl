#!/bin/bash

if [ $# -lt 1 ]; then
    echo "usage: $0 <tooldir>"
    echo "  tooldir: wasi-sdk base directory"
    exit 1
fi

set -x
set -e

TOOLDIR=$1
CC=${TOOLDIR}/bin/clang
AR=${TOOLDIR}/bin/llvm-ar
RANLIB=${TOOLDIR}/bin/llvm-ranlib
CFLAGS='-D_WASI_EMULATED_SIGNAL -D_WASI_EMULATED_GETPID -D_WASI_EMULATED_PROCESS_CLOCKS -mllvm -wasm-enable-sjlj -mllvm -wasm-use-legacy-eh=false'
LDFLAGS='-Wl,-z,stack-size=4194304'
LIBS='-lwasi-emulated-signal -lwasi-emulated-getpid -lwasi-emulated-process-clocks -lsetjmp'

export CC CFLAGS LDFLAGS LIBS AR RANLIB

HERE=`pwd`
PREFIX=${HERE}/sysroot

# Remove existing build directories before building.
rm -rf sdl-build prboom-build sdlquake-build
rm -rf sysroot

mkdir sdl-build
pushd sdl-build
CC=${CC} AR=${TOOLDIR}/bin/ar RANLIB=${TOOLDIR}/bin/ranlib ../SDL-1.2.15/configure --host=wasm32-wasip1 --prefix=${PREFIX}
make ${MAKEFLAGS}
make install
popd

mkdir prboom-build
pushd prboom-build
CC=${CC} AR=${AR} RANLIB=${RANLIB} ../prboom-2.5.0/configure --host=wasm32-wasip1 --disable-gl --with-sdl-prefix=${PREFIX} --prefix=${PREFIX}
make ${MAKEFLAGS}
make install
popd

mkdir sdlquake-build
pushd sdlquake-build
CC=${CC} AR=${AR} RANLIB=${RANLIB} ../sdlquake-1.0.9/configure --host=wasm32-wasip1 --disable-sdltest --with-sdl-prefix=${PREFIX} --prefix=${PREFIX}
make ${MAKEFLAGS}
make install
popd
