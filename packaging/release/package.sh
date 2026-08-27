#!/usr/bin/env sh
set -eu

if [ "$#" -ne 3 ]; then
    echo "usage: package.sh <version> <input-directory> <output-directory>" >&2
    exit 2
fi

version=$1
input_directory=$2
output_directory=$3

case "$version" in
    "" | *[!0-9A-Za-z.+-]*)
        echo "release version '$version' contains unsupported filename characters" >&2
        exit 2
        ;;
esac

if [ ! -d "$input_directory" ]; then
    echo "release input directory '$input_directory' does not exist" >&2
    exit 1
fi
if [ -z "$output_directory" ]; then
    echo "release output directory cannot be empty" >&2
    exit 2
fi
if [ -e "$output_directory" ]; then
    echo "release output path '$output_directory' already exists" >&2
    exit 1
fi

repository_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd -P)
license="$repository_root/LICENSE"
if [ ! -f "$license" ]; then
    echo "release license '$license' does not exist" >&2
    exit 1
fi

input_directory=$(CDPATH='' cd -- "$input_directory" && pwd -P)
output_name=$(basename -- "$output_directory")
output_parent=$(dirname -- "$output_directory")
mkdir -p "$output_parent"
output_parent=$(CDPATH='' cd -- "$output_parent" && pwd -P)
output_directory="$output_parent/$output_name"
if [ -e "$output_directory" ]; then
    echo "release output path '$output_directory' already exists" >&2
    exit 1
fi

temporary_directory=$(mktemp -d "$output_parent/.ragavan-release.XXXXXX")
artifact_directory="$temporary_directory/artifacts"
stage_directory="$temporary_directory/stage"
mkdir "$artifact_directory" "$stage_directory"

cleanup() {
    rm -rf -- "$temporary_directory"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

package() {
    source_directory=$1
    target=${source_directory##*/binary-}

    case "$target" in
        "" | *[!0-9A-Za-z._-]*)
            echo "release target '$target' contains unsupported filename characters" >&2
            exit 1
            ;;
    esac

    if [ -f "$source_directory/ragavan" ] && [ ! -f "$source_directory/ragavan.exe" ]; then
        binary=ragavan
        format=tar.gz
    elif [ -f "$source_directory/ragavan.exe" ] && [ ! -f "$source_directory/ragavan" ]; then
        binary=ragavan.exe
        format=zip
    else
        echo "release artifact '$source_directory' must contain exactly one recognized Ragavan binary" >&2
        exit 1
    fi

    source="$source_directory/$binary"
    root="ragavan-v$version-$target"
    stage="$stage_directory/$root"

    mkdir "$stage"
    cp "$source" "$stage/$binary"
    cp "$license" "$stage/LICENSE"

    case "$format" in
        tar.gz)
            chmod 755 "$stage/$binary"
            tar -C "$stage_directory" -czf "$artifact_directory/$root.tar.gz" "$root"
            ;;
        zip)
            (cd "$stage_directory" && zip -q -r "$artifact_directory/$root.zip" "$root")
            ;;
    esac
}

package_count=0
for source_directory in "$input_directory"/binary-*; do
    if [ ! -d "$source_directory" ]; then
        continue
    fi

    package "$source_directory"
    package_count=$((package_count + 1))
done

if [ "$package_count" -eq 0 ]; then
    echo "release input directory '$input_directory' contains no release artifacts" >&2
    exit 1
fi

(
    cd "$artifact_directory"
    sha256sum ragavan-* > SHA256SUMS
    sha256sum --check SHA256SUMS
)

mv -- "$artifact_directory" "$output_directory"
