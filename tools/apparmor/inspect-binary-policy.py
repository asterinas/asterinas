#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0

import argparse
import sys


TYPE_U8 = 0
TYPE_U16 = 1
TYPE_U32 = 2
TYPE_U64 = 3
TYPE_NAME = 4
TYPE_STRING = 5
TYPE_BLOB = 6
TYPE_STRUCT = 7
TYPE_STRUCT_END = 8
TYPE_ARRAY = 11
TYPE_ARRAY_END = 12

TYPE_NAMES = {
    TYPE_U8: "u8",
    TYPE_U16: "u16",
    TYPE_U32: "u32",
    TYPE_U64: "u64",
    TYPE_STRING: "string",
    TYPE_BLOB: "blob",
    TYPE_STRUCT: "struct",
    TYPE_ARRAY: "array",
}

LINUX_KERNEL_ABI_MASK = 0x3FF
LINUX_FORCE_COMPLAIN_FLAG = 1 << 11
YYTH_MAGIC = b"\x1b\x5e\x78\x3d"


class PolicyError(Exception):
    pass


class Reader:
    def __init__(self, data):
        self.data = data
        self.offset = 0

    def has_remaining(self):
        return self.offset < len(self.data)

    def peek_u8(self):
        if not self.has_remaining():
            return None
        return self.data[self.offset]

    def read_bytes(self, length):
        end = self.offset + length
        if end > len(self.data):
            raise PolicyError("policy stream is truncated")
        value = self.data[self.offset:end]
        self.offset = end
        return value

    def read_u8_raw(self):
        return self.read_bytes(1)[0]

    def read_u16_raw(self):
        return int.from_bytes(self.read_bytes(2), "little")

    def read_u32_raw(self):
        return int.from_bytes(self.read_bytes(4), "little")

    def read_u64_raw(self):
        return int.from_bytes(self.read_bytes(8), "little")

    def read_nul_string(self, length):
        data = self.read_bytes(length)
        if not data or data[-1] != 0:
            raise PolicyError("string is not nul-terminated")
        text = data[:-1].decode("utf-8")
        if "\0" in text:
            raise PolicyError("string contains an embedded nul byte")
        return text

    def read_name(self):
        length = self.read_u16_raw()
        return self.read_nul_string(length)

    def read_optional_name(self):
        if self.peek_u8() != TYPE_NAME:
            return None
        self.offset += 1
        return self.read_name()

    def read_field(self):
        name = self.read_optional_name()
        field_type = self.read_u8_raw()
        if field_type == TYPE_STRUCT_END or field_type == TYPE_ARRAY_END:
            raise PolicyError("unexpected policy container terminator")
        value = self.read_value(field_type)
        return {"name": name, "type": field_type, "value": value}

    def read_value(self, field_type):
        if field_type == TYPE_U8:
            return self.read_u8_raw()
        if field_type == TYPE_U16:
            return self.read_u16_raw()
        if field_type == TYPE_U32:
            return self.read_u32_raw()
        if field_type == TYPE_U64:
            return self.read_u64_raw()
        if field_type == TYPE_STRING:
            return self.read_nul_string(self.read_u16_raw())
        if field_type == TYPE_BLOB:
            return self.read_bytes(self.read_u32_raw())
        if field_type == TYPE_STRUCT:
            fields = []
            while self.peek_u8() != TYPE_STRUCT_END:
                if self.peek_u8() is None:
                    raise PolicyError("struct is not terminated")
                fields.append(self.read_field())
            self.offset += 1
            return fields
        if field_type == TYPE_ARRAY:
            declared_count = self.read_u16_raw()
            fields = []
            while self.peek_u8() != TYPE_ARRAY_END:
                if self.peek_u8() is None:
                    raise PolicyError("array is not terminated")
                fields.append(self.read_field())
            if self.peek_u8() != TYPE_ARRAY_END:
                raise PolicyError("array is not terminated")
            self.offset += 1
            return {"declared_count": declared_count, "items": fields}
        raise PolicyError(f"unsupported policy field type {field_type}")


def parse_policy(data):
    reader = Reader(data)
    fields = []
    while reader.has_remaining():
        fields.append(reader.read_field())
    return fields


def walk_fields(fields):
    for field in fields:
        yield field
        value = field["value"]
        if field["type"] == TYPE_STRUCT:
            yield from walk_fields(value)
        elif field["type"] == TYPE_ARRAY:
            yield from walk_fields(value["items"])


def find_named(fields, name):
    return [field for field in walk_fields(fields) if field["name"] == name]


def decode_abi_version(raw_version):
    if raw_version <= 5:
        return raw_version
    return raw_version & LINUX_KERNEL_ABI_MASK


def type_name(field):
    return TYPE_NAMES.get(field["type"], str(field["type"]))


def profile_name(profile_field):
    for child in profile_field["value"]:
        if child["name"] is None and child["type"] == TYPE_STRING:
            return child["value"]
    raise PolicyError("profile struct does not contain a profile name")


def profile_mode(profile_field, force_complain):
    for child in profile_field["value"]:
        if child["name"] == "flags" and child["type"] == TYPE_STRUCT:
            raw_values = [
                item["value"]
                for item in child["value"]
                if item["name"] is None and item["type"] == TYPE_U32
            ]
            if len(raw_values) < 2:
                raise PolicyError("profile flags are incomplete")
            packed_mode = raw_values[1]
            if force_complain or packed_mode == 1:
                return "complain"
            if packed_mode == 0:
                return "enforce"
            return f"unsupported({packed_mode})"
    raise PolicyError("profile flags are missing")


def profile_has_file_dfa(profile_field):
    blobs = [
        field["value"]
        for field in walk_fields(profile_field["value"])
        if field["name"] == "aadfa" and field["type"] == TYPE_BLOB
    ]
    for blob in blobs:
        if blob.find(YYTH_MAGIC) >= 0:
            return True
    return False


def profile_perm_rows(profile_field):
    for field in walk_fields(profile_field["value"]):
        if field["name"] != "perms" or field["type"] != TYPE_STRUCT:
            continue
        for child in field["value"]:
            if child["type"] == TYPE_ARRAY:
                return child["value"]["declared_count"]
    return 0


def inspect(path):
    data = path.read_bytes()
    fields = parse_policy(data)
    versions = find_named(fields, "version")
    if not versions or versions[0]["type"] != TYPE_U32:
        raise PolicyError("policy stream does not contain a Linux AppArmor version")

    raw_version = versions[0]["value"]
    force_complain = bool(raw_version & LINUX_FORCE_COMPLAIN_FLAG)
    profiles = [
        field
        for field in fields
        if field["name"] == "profile" and field["type"] == TYPE_STRUCT
    ]
    if not profiles:
        raise PolicyError("policy stream does not contain profiles")

    print(f"format: linux-apparmor-typed-stream")
    print(f"size: {len(data)} bytes")
    print(f"abi_version: {decode_abi_version(raw_version)}")
    print(f"force_complain: {str(force_complain).lower()}")
    print(f"profiles: {len(profiles)}")
    for profile in profiles:
        name = profile_name(profile)
        mode = profile_mode(profile, force_complain)
        has_dfa = profile_has_file_dfa(profile)
        perm_rows = profile_perm_rows(profile)
        print(
            f"- name: {name}, mode: {mode}, file_dfa: "
            f"{str(has_dfa).lower()}, perm_rows: {perm_rows}"
        )

    unknown_top_level = [
        field["name"] or type_name(field)
        for field in fields
        if field["name"] not in ("version", "namespace", "profile")
    ]
    if unknown_top_level:
        print("unknown_top_level: " + ", ".join(unknown_top_level))


def main():
    parser = argparse.ArgumentParser(
        description="Inspects Linux AppArmor binary policy output for Asterinas."
    )
    parser.add_argument("policy", type=argparse.FileType("rb"))
    args = parser.parse_args()

    class PathAdapter:
        def __init__(self, file_object):
            self.file_object = file_object

        def read_bytes(self):
            return self.file_object.read()

    try:
        inspect(PathAdapter(args.policy))
    except PolicyError as error:
        print(f"inspect-binary-policy.py: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
