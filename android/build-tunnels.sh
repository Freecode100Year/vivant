#!/usr/bin/env bash
# Builds the bundled tunnel tools for Android from their official sources:
#   easytier-core (EasyTier, Rust) -> app/src/main/jniLibs/<abi>/libeasytier.so
#   frpc          (frp, Go + cgo)  -> app/src/main/jniLibs/<abi>/libfrpc.so  (no admin web UI)
# Like libvivant.so these are plain executables, named .so so Android
# extracts them where an app may run them.
#
# 从官方源码为安卓编译 EasyTier 和 frpc。
#
#   ANDROID_NDK_HOME=… android/build-tunnels.sh easytier|frpc [abi ...]
# Needs: git, cargo + rustup (EasyTier pins its own toolchain), protoc, Go.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=/dev/null
. "$here/tunnels.env"
tool=${1:?usage: build-tunnels.sh easytier|frpc [abi ...]}
shift
NDK=${ANDROID_NDK_HOME:-${ANDROID_NDK_LATEST_HOME:-${ANDROID_NDK_ROOT:-}}}
[ -d "$NDK" ] || { echo "set ANDROID_NDK_HOME" >&2; exit 1; }
API=${ANDROID_API:-26}
TC=$NDK/toolchains/llvm/prebuilt/$(uname -s | tr '[:upper:]' '[:lower:]')-x86_64/bin
abis=("$@")
[ ${#abis[@]} -gt 0 ] || abis=(arm64-v8a armeabi-v7a x86_64)
work=${TUNNEL_SRC:-$here/.tunnel-src}
mkdir -p "$work"

fetch() { # repo tag dir
	if [ ! -d "$work/$3/.git" ]; then
		git clone -q --depth 1 --branch "$2" "https://github.com/$1" "$work/$3"
	fi
	[ "$(git -C "$work/$3" describe --tags --exact-match)" = "$2" ] || { echo "$3 is not at $2" >&2; exit 1; }
}

for abi in "${abis[@]}"; do
	case $abi in
	arm64-v8a) triple=aarch64-linux-android clang=aarch64-linux-android$API-clang goarch=arm64 goarm= ;;
	armeabi-v7a) triple=armv7-linux-androideabi clang=armv7a-linux-androideabi$API-clang goarch=arm goarm=7 ;;
	x86_64) triple=x86_64-linux-android clang=x86_64-linux-android$API-clang goarch=amd64 goarm= ;;
	*) echo "unknown abi $abi" >&2; exit 1 ;;
	esac
	out=$here/app/src/main/jniLibs/$abi
	mkdir -p "$out"
	case $tool in
	easytier)
		fetch EasyTier/EasyTier "$EASYTIER_VERSION" easytier
		env_triple=$(echo "$triple" | tr '[:lower:]-' '[:upper:]_')
		under=${triple//-/_}
		(
			cd "$work/easytier"
			rustup target add "$triple" >/dev/null
			export ANDROID_NDK_ROOT=$NDK ANDROID_NDK_HOME=$NDK
			export "CARGO_TARGET_${env_triple}_LINKER=$TC/$clang" "CC_${under}=$TC/$clang" "CXX_${under}=$TC/$clang++" "AR_${under}=$TC/llvm-ar"
			cargo build --release --locked -p easytier --bin easytier-core --target "$triple"
		)
		cp "$work/easytier/target/$triple/release/easytier-core" "$out/libeasytier.so"
		"$TC/llvm-strip" "$out/libeasytier.so"
		;;
	frpc)
		fetch fatedier/frp "$FRP_VERSION" frp
		(
			cd "$work/frp"
			CGO_ENABLED=1 GOOS=android GOARCH=$goarch GOARM=$goarm CC=$TC/$clang \
				go build -trimpath -ldflags "-s -w" -tags frpc,noweb -o "$out/libfrpc.so" ./cmd/frpc
		)
		;;
	*) echo "unknown tool $tool" >&2; exit 1 ;;
	esac
	echo "built $tool $abi"
done
