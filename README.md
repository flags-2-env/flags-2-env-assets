# flags-2-env-assets

Versioned product images, CSS, CDN exports, and static assets; app branding is isolated from marketing sites.

## Image integrity

`tools/validate_assets.rs` is an offline, dependency-free Rust structural
validator for `branding/**/*.png`. It verifies PNG signatures, bounded
dimensions/size, IHDR ordering, chunk integrity with CRC32, IDAT presence,
IEND termination, and absence of trailing data. Symlinks in the scanned tree
fail closed. The pinned, read-only `png-asset-integrity.yml` workflow runs the
unit tests and checks committed assets at the exact PR head.

This check does **not** decompress pixels, scan image metadata for secrets,
validate content licensing, or replace human visual review. Changes to logos
remain separately reviewable.
