#!/usr/bin/env bash
# Builds the bundled frp client for Android from its official source:
#   frpc (frp, Go + cgo) -> app/src/main/jniLibs/<abi>/libfrpc.so  (no admin web UI)
# Like libvivant.so it is a plain executable, named .so so Android extracts
# it where an app may run it. (Tor comes prebuilt from tor-android.)
#
# 从官方源码为安卓编译 frpc。
#
#   ANDROID_NDK_HOME=… android/build-tunnels.sh frpc [abi ...]
# Needs: git, Go.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=/dev/null
. "$here/tunnels.env"
tool=${1:?usage: build-tunnels.sh frpc [abi ...]}
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
