#!/usr/bin/env bash
# Ensure libzvec_c_api is linkable. zvec-rust 0.7.x ships prebuilts for
# aarch64-apple-darwin / linux / windows but not x86_64-apple-darwin; Intel
# macOS must fall back to a CMake build from the upstream zvec sources.
set -euo pipefail

target="${1:-${TARGET:-}}"
if [[ -z "$target" ]]; then
  target="$(rustc -vV | awk '/^host:/{print $2; exit}')"
fi
version="${ZVEC_RUST_VERSION:-0.7.0}"
cache_root="${ZVEC_CACHE_DIR:-${RUNNER_TEMP:-/tmp}/a3s-zvec}"
outdir="${cache_root}/${target}"
mkdir -p "$outdir"

has_lib() {
  local dir="$1"
  case "$target" in
    *-apple-darwin|*-apple-ios)
      [[ -f "$dir/libzvec_c_api.dylib" ]]
      ;;
    *-pc-windows-*)
      [[ -f "$dir/zvec_c_api.lib" || -f "$dir/zvec_c_api.dll" ]]
      ;;
    *)
      [[ -f "$dir/libzvec_c_api.so" ]]
      ;;
  esac
}

if has_lib "$outdir"; then
  echo "Reusing cached zvec native library in $outdir"
else
  url="https://github.com/zvec-ai/zvec-rust/releases/download/v${version}/zvec-prebuilt-${target}.tar.gz"
  echo "Trying prebuilt zvec from $url"
  if curl -fsSL --retry 3 --retry-delay 2 -A "a3s-cli-zvec-provision/1.0" \
    -o "${outdir}/prebuilt.tar.gz" "$url"; then
    tar -xzf "${outdir}/prebuilt.tar.gz" -C "$outdir"
    rm -f "${outdir}/prebuilt.tar.gz"
  else
    echo "Prebuilt unavailable for $target; building zvec C API from source"
    src="${cache_root}/src/zvec"
    build="${cache_root}/cmake-build-${target}"
    rm -rf "$build"
    mkdir -p "$build"
    if [[ ! -f "$src/CMakeLists.txt" ]]; then
      rm -rf "$src"
      git clone --depth 1 --recurse-submodules --shallow-submodules \
        https://github.com/alibaba/zvec.git "$src"
    fi
    cmake_args=(
      -S "$src"
      -B "$build"
      -DCMAKE_BUILD_TYPE=Release
      -DBUILD_C_BINDINGS=ON
      -DBUILD_TOOLS=OFF
    )
    case "$target" in
      x86_64-apple-darwin)
        cmake_args+=(-DCMAKE_OSX_ARCHITECTURES=x86_64 -DCMAKE_OSX_DEPLOYMENT_TARGET=12.0)
        ;;
      aarch64-apple-darwin)
        cmake_args+=(-DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_OSX_DEPLOYMENT_TARGET=12.0)
        ;;
    esac
    cmake "${cmake_args[@]}"
    cmake --build "$build" --config Release -j "$(sysctl -n hw.ncpu 2>/dev/null || nproc)"
    found=""
    for candidate in \
      "$build/lib" \
      "$build/lib/Release" \
      "$build/Release" \
      "$build/src/binding/c" \
      "$build/src/binding/c/Release"; do
      if has_lib "$candidate"; then
        found="$candidate"
        break
      fi
    done
    if [[ -z "$found" ]]; then
      echo "error: cmake build finished but libzvec_c_api was not found" >&2
      find "$build" -name '*zvec_c_api*' | head -50 >&2 || true
      exit 1
    fi
    cp -R "$found"/. "$outdir"/
  fi
fi

if ! has_lib "$outdir"; then
  echo "error: zvec native library missing under $outdir" >&2
  ls -la "$outdir" >&2 || true
  exit 1
fi

include_dir=""
for candidate in \
  "${cache_root}/src/zvec/src/include" \
  "$outdir/include" \
  "$outdir/../include"; do
  if [[ -d "$candidate" ]]; then
    include_dir="$candidate"
    break
  fi
done

{
  echo "ZVEC_LIB_DIR=$outdir"
  if [[ -n "$include_dir" ]]; then
    echo "ZVEC_INCLUDE_DIR=$include_dir"
  fi
  echo "LIBRARY_PATH=${outdir}${LIBRARY_PATH:+:$LIBRARY_PATH}"
  echo "DYLD_LIBRARY_PATH=${outdir}${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
  echo "LD_LIBRARY_PATH=${outdir}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
} >> "${GITHUB_ENV:-/dev/stdout}"

echo "Provisioned zvec for $target at $outdir"
