# SPDX-License-Identifier: MPL-2.0

"""Apply the TCG settings while preserving Nixpkgs' Kata defaults and paths."""

import json
import sys
import tomllib

import tomli_w


def merge_settings(defaults, overrides):
    for key, value in overrides.items():
        if isinstance(value, dict):
            merge_settings(defaults[key], value)
        else:
            defaults[key] = value


with open(sys.argv[1], "rb") as source:
    configuration = tomllib.load(source)
with open(sys.argv[2]) as source:
    merge_settings(configuration, json.load(source))
with open(sys.argv[3], "wb") as output:
    tomli_w.dump(configuration, output)
