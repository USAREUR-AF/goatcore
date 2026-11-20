#!/usr/bin/env bash
# Download and verify the single Ubuntu image selected by 00-prerequisites.sh.
# The caller supplies info(), ok(), and error() for consistent output.

image_sha256() {
    local image="$1" output digest

    if command -v sha256sum >/dev/null 2>&1; then
        output="$(sha256sum "$image")" || return 1
        digest="${output%%[[:space:]]*}"
    elif command -v shasum >/dev/null 2>&1; then
        output="$(shasum -a 256 "$image")" || return 1
        digest="${output%%[[:space:]]*}"
    elif command -v openssl >/dev/null 2>&1; then
        output="$(openssl dgst -sha256 "$image")" || return 1
        digest="${output##* }"
    else
        error "No SHA-256 tool found. Install sha256sum (coreutils), shasum, or openssl, then re-run."
        return 1
    fi

    digest="$(printf '%s' "$digest" | tr '[:upper:]' '[:lower:]')"
    if [ "${#digest}" -ne 64 ] || printf '%s' "$digest" | grep -q '[^0-9a-f]'; then
        error "The available SHA-256 tool returned a malformed digest for $image"
        return 1
    fi
    printf '%s\n' "$digest"
}

verify_ubuntu_image() {
    local image="$1" expected="$2" actual

    actual="$(image_sha256 "$image")" || {
        error "Could not calculate SHA-256 for $image"
        return 1
    }
    if [ "$actual" != "$expected" ]; then
        error "SHA-256 mismatch for $image"
        error "Expected: $expected"
        error "Actual:   $actual"
        return 1
    fi
    ok "SHA-256 verified: $(basename "$image")"
}

prepare_ubuntu_image() {
    local download_dir="$1" fallback_dir="$2" release_url="$3" image_name="$4" expected="$5"
    local image_path image_url partial

    if [ "${#expected}" -ne 64 ] || [[ "$expected" == *[!0123456789abcdef]* ]]; then
        error "Invalid pinned SHA-256 for $image_name"
        return 1
    fi

    if [ -f "$download_dir/$image_name" ]; then
        image_path="$download_dir/$image_name"
        ok "Found image: $image_name"
    elif [ "$fallback_dir" != "$download_dir" ] && [ -f "$fallback_dir/$image_name" ]; then
        image_path="$fallback_dir/$image_name"
        ok "Found image: $image_name"
    else
        command -v curl >/dev/null 2>&1 || {
            error "curl not found. Install curl, then re-run ./00-prerequisites.sh"
            return 1
        }
        image_url="${release_url}/${image_name}"
        mkdir -p "$download_dir" || {
            error "Could not create image cache directory: $download_dir"
            return 1
        }
        image_path="$download_dir/$image_name"
        partial="${image_path}.part.$$"
        info "Downloading Ubuntu Server image $image_name..."
        if ! curl --fail --location --progress-bar --output "$partial" "$image_url"; then
            error "Failed to download Ubuntu image: $image_url"
            error "The partial download was removed; re-run when the download is available."
            rm -f "$partial"
            return 1
        fi
        if ! verify_ubuntu_image "$partial" "$expected"; then
            error "The failed download was removed and was not added to the image cache."
            rm -f "$partial"
            return 1
        fi
        if ! mv "$partial" "$image_path"; then
            error "Verified image could not be moved into the cache: $image_path"
            rm -f "$partial"
            return 1
        fi
        ok "Downloaded and verified: $image_name"
        # Public result consumed by 00-prerequisites.sh after this helper returns.
        # shellcheck disable=SC2034
        IMAGE_PATH="$image_path"
        return 0
    fi

    if ! verify_ubuntu_image "$image_path" "$expected"; then
        error "The cached file was left in place; remove or replace only that image before re-running."
        return 1
    fi
    # Public result consumed by 00-prerequisites.sh after this helper returns.
    # shellcheck disable=SC2034
    IMAGE_PATH="$image_path"
}
