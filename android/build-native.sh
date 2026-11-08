#!/usr/bin/env bash
# Cross-compiles the VIVANT server for Android with the NDK and puts it where
# the APK picks it up: app/src/main/jniLibs/<abi>/libvivant.so. It is a normal
# executable, only named like a library so Android extracts it to the app's
# nativeLibraryDir, the one place an app may run programs from.
#
# 用 NDK 交叉编译服务器，放进 APK 的 jniLibs。
#
#   ANDROID_NDK_HOME=/path/to/ndk android/build-native.sh [abi ...]
#   abi: arm64-v8a (default all), armeabi-v7a, x86_64
set -euo pipefail
cd "$(dirname "$0")/.."
NDK=${ANDROID_NDK_HOME:-${ANDROID_NDK_LATEST_HOME:-${ANDROID_NDK_ROOT:-}}}
[ -d "$NDK" ] || { echo "set ANDROID_NDK_HOME to an Android NDK (r25+)" >&2; exit 1; }
API=${ANDROID_API:-26}
host=$(uname -s | tr '[:upper:]' '[:lower:]')-x86_64
TC=$NDK/toolchains/llvm/prebuilt/$host/bin
[ -d "$TC" ] || { echo "no NDK toolchain at $TC" >&2; exit 1; }
abis=("$@")
[ ${#abis[@]} -gt 0 ] || abis=(arm64-v8a armeabi-v7a x86_64)

for abi in "${abis[@]}"; do
	case $abi in
	arm64-v8a) triple=aarch64-linux-android clang=aarch64-linux-android$API-clang ;;
	armeabi-v7a) triple=armv7-linux-androideabi clang=armv7a-linux-androideabi$API-clang ;;
	x86_64) triple=x86_64-linux-android clang=x86_64-linux-android$API-clang ;;
	*) echo "unknown abi $abi" >&2; exit 1 ;;
	esac
	env_triple=$(echo "$triple" | tr '[:lower:]-' '[:upper:]_')
	under=${triple//-/_}
	export "CARGO_TARGET_${env_triple}_LINKER=$TC/$clang"
	export "CC_${under}=$TC/$clang"
	export "AR_${under}=$TC/llvm-ar"
	cargo build --release --locked --bin vivant --target "$triple"
	mkdir -p "android/app/src/main/jniLibs/$abi"
	cp "target/$triple/release/vivant" "android/app/src/main/jniLibs/$abi/libvivant.so"
	echo "built $abi"
done
