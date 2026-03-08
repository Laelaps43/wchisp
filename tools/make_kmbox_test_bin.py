#!/usr/bin/env python3
"""
Build controlled kmbox test firmware images from a known-good base image.

This is intended for bootloader behavior experiments, not for producing a
bootable custom application from scratch.
"""

from __future__ import annotations

import argparse
import pathlib
import sys


BLOCK_SIZE = 60


def parse_u32(text: str) -> int:
    if text.startswith(("0x", "0X")):
        return int(text[2:], 16)
    return int(text, 10)


def block_offset(block_index: int) -> int:
    return block_index * BLOCK_SIZE


def patch_block(data: bytearray, block_index: int, pattern: bytes) -> None:
    off = block_offset(block_index)
    if off + BLOCK_SIZE > len(data):
        raise ValueError(f"block {block_index} out of range")
    buf = (pattern * ((BLOCK_SIZE + len(pattern) - 1) // len(pattern)))[:BLOCK_SIZE]
    data[off : off + BLOCK_SIZE] = buf


def swap_blocks(data: bytearray, a: int, b: int) -> None:
    oa = block_offset(a)
    ob = block_offset(b)
    if oa + BLOCK_SIZE > len(data) or ob + BLOCK_SIZE > len(data):
        raise ValueError("swap block out of range")
    ba = bytes(data[oa : oa + BLOCK_SIZE])
    bb = bytes(data[ob : ob + BLOCK_SIZE])
    data[oa : oa + BLOCK_SIZE] = bb
    data[ob : ob + BLOCK_SIZE] = ba


def main() -> int:
    parser = argparse.ArgumentParser(description="Create controlled kmbox test firmware")
    parser.add_argument(
        "--base",
        default="kmboxNet固件20260212_12h34m17s.bin",
        help="base firmware image",
    )
    parser.add_argument("--output", required=True, help="output firmware image")

    sub = parser.add_subparsers(dest="mode", required=True)

    p_patch = sub.add_parser("patch-block", help="replace one 60-byte block with a pattern")
    p_patch.add_argument("--block", required=True, type=parse_u32, help="block index")
    p_patch.add_argument(
        "--pattern",
        required=True,
        help="ASCII pattern, repeated to 60 bytes (example: CODEx80)",
    )

    p_swap = sub.add_parser("swap-blocks", help="swap two 60-byte blocks")
    p_swap.add_argument("--block-a", required=True, type=parse_u32)
    p_swap.add_argument("--block-b", required=True, type=parse_u32)

    p_fill = sub.add_parser("fill-range", help="fill a block range with a pattern")
    p_fill.add_argument("--start-block", required=True, type=parse_u32)
    p_fill.add_argument("--count", required=True, type=parse_u32)
    p_fill.add_argument("--pattern", required=True)

    args = parser.parse_args()

    base_path = pathlib.Path(args.base)
    out_path = pathlib.Path(args.output)

    if not base_path.exists():
        print(f"base firmware not found: {base_path}", file=sys.stderr)
        return 2

    data = bytearray(base_path.read_bytes())
    total_blocks = len(data) // BLOCK_SIZE

    if args.mode == "patch-block":
        patch_block(data, args.block, args.pattern.encode("ascii"))
        desc = f"patched block {args.block}"
    elif args.mode == "swap-blocks":
        swap_blocks(data, args.block_a, args.block_b)
        desc = f"swapped blocks {args.block_a} and {args.block_b}"
    elif args.mode == "fill-range":
        for i in range(args.start_block, args.start_block + args.count):
            patch_block(data, i, args.pattern.encode("ascii"))
        desc = f"filled {args.count} block(s) from {args.start_block}"
    else:
        raise AssertionError("unreachable")

    out_path.write_bytes(data)
    print(f"wrote {out_path} ({len(data)} bytes, {total_blocks} blocks): {desc}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
