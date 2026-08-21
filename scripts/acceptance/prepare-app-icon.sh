#!/bin/zsh
set -euo pipefail

source_path="${BAGENT_ICON_SOURCE:-/Users/oliver/Programming/bagent/bagent_icon.png}"
expected_hash="8e38f48a25e179ad6e4bc3cff19d950563bb14868924743eefe1d4d87e673303"
output_dir="${1:?output directory required}"

[[ -f "$source_path" ]] || { echo "approved icon source is missing" >&2; exit 2; }
actual_hash="$(shasum -a 256 "$source_path" | awk '{print $1}')"
[[ "$actual_hash" == "$expected_hash" ]] || { echo "approved icon source hash mismatch" >&2; exit 2; }

rm -rf "$output_dir"
mkdir -p "$output_dir/AppIcon.iconset"
sips -s format png -z 1024 1024 "$source_path" --out "$output_dir/bagent-permission.png" >/dev/null

for size in 16 32 128 256 512 1024; do
    sips -z "$size" "$size" "$output_dir/bagent-permission.png" \
        --out "$output_dir/AppIcon.iconset/icon_${size}x${size}.png" >/dev/null
    if [[ "$size" -lt 1024 ]]; then
        double_size=$((size * 2))
        sips -z "$double_size" "$double_size" "$output_dir/bagent-permission.png" \
            --out "$output_dir/AppIcon.iconset/icon_${size}x${size}@2x.png" >/dev/null
    fi
done

iconutil -c icns "$output_dir/AppIcon.iconset" -o "$output_dir/AppIcon.icns"
shasum -a 256 "$output_dir/bagent-permission.png" "$output_dir/AppIcon.icns"
