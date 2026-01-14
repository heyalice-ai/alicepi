#!/bin/bash

set -e

# ARCH=aarch64-unknown-linux-musl

DEPLOY_TARGET=${DEPLOY_TARGET:-alicedev1}
ARCH=aarch64-unknown-linux-gnu
PROFILE=release

SYSROOT="$(aarch64-linux-gnu-gcc -print-sysroot)"
SYSROOT_INCLUDE="${SYSROOT}/usr/include"
SYSROOT_LIB64="${SYSROOT}/usr/lib64"
SYSROOT_LIB="${SYSROOT}/lib64"
GCC_SYSROOT_INCLUDE="$(find "${SYSROOT}/usr/lib/gcc" -type d -path "*/include" 2>/dev/null | sort -V | tail -n 1)"
SYSROOT_CXX_INCLUDE="${SYSROOT}/usr/include/c++"
SYSROOT_CXX_VERSION_INCLUDE="${SYSROOT}/usr/include/c++/15"
SYSROOT_CXX_TARGET_INCLUDE="${SYSROOT}/usr/include/c++/15/aarch64-redhat-linux"
export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_SYSROOT_DIR_aarch64_unknown_linux_gnu="${SYSROOT}"
export PKG_CONFIG_LIBDIR_aarch64_unknown_linux_gnu="${SYSROOT_LIB64}/pkgconfig:${SYSROOT}/usr/share/pkgconfig"
export PKG_CONFIG_PATH_aarch64_unknown_linux_gnu="${PKG_CONFIG_LIBDIR_aarch64_unknown_linux_gnu}"

export BINDGEN_EXTRA_CLANG_ARGS="--target=${ARCH} --sysroot=${SYSROOT} -isystem ${SYSROOT_INCLUDE}"
if [ -n "${GCC_SYSROOT_INCLUDE}" ]; then
  export BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS} -isystem ${GCC_SYSROOT_INCLUDE}"
fi
if [ -d "${SYSROOT_CXX_VERSION_INCLUDE}" ]; then
  export BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS} -isystem ${SYSROOT_CXX_VERSION_INCLUDE}"
fi
if [ -d "${SYSROOT_CXX_TARGET_INCLUDE}" ]; then
  export BINDGEN_EXTRA_CLANG_ARGS="${BINDGEN_EXTRA_CLANG_ARGS} -isystem ${SYSROOT_CXX_TARGET_INCLUDE}"
fi
export CFLAGS="--sysroot=${SYSROOT} -I${SYSROOT_INCLUDE}"
export CXXFLAGS="--sysroot=${SYSROOT} -I${SYSROOT_INCLUDE} -I${SYSROOT_CXX_VERSION_INCLUDE} -I${SYSROOT_CXX_TARGET_INCLUDE}"
export LDFLAGS="--sysroot=${SYSROOT} -L${SYSROOT_LIB64} -L${SYSROOT_LIB}"
export CMAKE_EXE_LINKER_FLAGS="--sysroot=${SYSROOT} -L${SYSROOT_LIB64} -L${SYSROOT_LIB}"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C force-frame-pointers=yes -C link-arg=-Wl,--no-as-needed -C link-arg=-L/usr/lib/gcc/aarch64-linux-gnu/15 -C link-arg=-lgcc -C link-arg=-l:libatomic.so.1 -C link-arg=-Wl,--as-needed"

if [ -e "${SYSROOT}/lib64/libgcc_s.so.1" ] && [ ! -e "${SYSROOT}/lib64/libgcc_s.so" ]; then
  ln -s libgcc_s.so.1 "${SYSROOT}/lib64/libgcc_s.so"
fi
if [ -e "${SYSROOT}/usr/lib64/libstdc++.so.6" ] && [ ! -e "${SYSROOT}/usr/lib64/libstdc++.so" ]; then
  ln -s libstdc++.so.6 "${SYSROOT}/usr/lib64/libstdc++.so"
fi
if [ -e "${SYSROOT}/usr/lib64/libatomic.so.1" ] && [ ! -e "${SYSROOT}/usr/lib64/libatomic.so" ]; then
  ln -s libatomic.so.1 "${SYSROOT}/usr/lib64/libatomic.so"
fi


# cargo build --features=gpio --profile=${PROFILE} --target ${ARCH} "$@"
cargo build --profile=${PROFILE} --target ${ARCH} "$@"

rsync -rvpP ./target/${ARCH}/${PROFILE}/alicepi ${DEPLOY_TARGET}:alicepi/alicepi-static


# Also sync the models
rsync -rvpP ./models ./assets ${DEPLOY_TARGET}:alicepi/

