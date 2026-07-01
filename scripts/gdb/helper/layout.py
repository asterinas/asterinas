# SPDX-License-Identifier: MPL-2.0

"""
Rust layout readers for the GDB Python API.

This module is the only place that understands generic Rust storage
wrappers such as ``Arc``, ``Weak``, ``Option``, ``Vec``, ``BTreeMap``,
``Atomic<T>``, and the ostd lock wrappers.  Higher-level helpers use
these readers instead of spelling out noisy DWARF paths themselves.
"""

import re

import gdb


class IntrospectionError(Exception):
    """Raised when a type introspection operation fails."""

    pass


_type_cache = {}


def _resolve_symbol_value(symbol_name):
    """Resolve a Rust symbol using the GDB mechanisms that survive GDB 15."""
    try:
        sym = gdb.lookup_static_symbol(symbol_name)
        if sym is not None:
            return sym.value()
    except (gdb.error, AttributeError):
        pass

    try:
        return gdb.parse_and_eval(f"&{symbol_name}").dereference()
    except gdb.error:
        pass

    try:
        addr_out = gdb.execute(f"info address {symbol_name}", to_string=True)
        match = re.search(r"(0x[0-9a-fA-F]+)", addr_out)
        if match is None:
            return None

        type_out = gdb.execute(f"ptype {symbol_name}", to_string=True)
        type_match = re.match(r"type\s*=\s*(.*)", type_out.strip(), re.DOTALL)
        if type_match is None:
            return None

        type_str = type_match.group(1).strip().rstrip(";")
        return gdb.parse_and_eval(f"*({type_str} *){match.group(1)}")
    except gdb.error:
        return None


def lookup_type(type_name):
    """Look up a Rust type name, caching successful lookups."""
    if type_name in _type_cache:
        return _type_cache[type_name]
    try:
        rust_type = gdb.lookup_type(type_name)
        _type_cache[type_name] = rust_type
        return rust_type
    except gdb.error:
        return None


def read_global(symbol_name):
    """Read a global symbol value."""
    return _resolve_symbol_value(symbol_name)


def _get_pp(val):
    try:
        return gdb.default_visualizer(val)
    except Exception:
        return None


def _pp_children(pp):
    try:
        return list(pp.children())
    except Exception:
        return None


def _pp_to_string(pp):
    try:
        result = pp.to_string()
        if result is None:
            return None
        if isinstance(result, str):
            return result
        if hasattr(result, 'value'):
            length = result.length
            if length == 0:
                return ""
            encoding = result.encoding or "utf-8"
            return result.value().string(
                encoding, errors="replace", length=length
            )
        return str(result)
    except Exception:
        return None


def _read_integer_leaf(value, depth=0):
    if depth > 8:
        raise IntrospectionError("nested scalar wrapper is too deep")

    try:
        return int(value)
    except (gdb.error, TypeError, ValueError):
        pass

    for field_name in ('__0', '0', 'value', 'v'):
        try:
            return _read_integer_leaf(value[field_name], depth + 1)
        except (gdb.error, KeyError, IntrospectionError):
            continue

    try:
        fields = value.type.fields()
    except (gdb.error, AttributeError):
        fields = []
    if len(fields) == 1:
        try:
            return _read_integer_leaf(value[fields[0]], depth + 1)
        except (gdb.error, KeyError, IntrospectionError):
            pass

    raise IntrospectionError(f"Failed to read integer from {value.type}")


def unwrap_atomic(val):
    """Unwrap a current Rust ``Atomic<T>`` scalar to its inner integer."""
    try:
        return _read_integer_leaf(val['v']['value'])
    except (gdb.error, KeyError, IntrospectionError):
        pass
    raise IntrospectionError("Failed to unwrap atomic value")


def read_scalar(value):
    """Read an integer-like scalar, handling atomics and tuple structs."""
    try:
        return int(value)
    except (gdb.error, TypeError, ValueError):
        pass
    try:
        return unwrap_atomic(value)
    except IntrospectionError:
        pass
    return _read_integer_leaf(value)


def unwrap_mutex(mutex_val):
    """Return ``(locked, value)`` for an ostd ``Mutex<T>``."""
    try:
        locked = bool(unwrap_atomic(mutex_val['lock']))
        return (locked, mutex_val['val']['value'])
    except (gdb.error, IntrospectionError) as error:
        raise IntrospectionError(f"Failed to unwrap Mutex: {error}")


def unwrap_rwmutex_value(rwmutex_val):
    """Return the inner value of an ostd ``RwMutex<T>``."""
    try:
        return rwmutex_val['val']['value']
    except gdb.error as error:
        raise IntrospectionError(f"Failed to unwrap RwMutex: {error}")


def _require_pp(val, type_name):
    pp = _get_pp(val)
    if pp is None:
        raise IntrospectionError(
            f"No rust-gdb printer for {type_name}. "
            "Use rust-gdb instead of plain gdb."
        )
    return pp


def unwrap_arc(arc_val):
    """Dereference ``Arc<T>`` to ``T`` via the rust-gdb printer."""
    children = _pp_children(_require_pp(arc_val, "Arc"))
    if children is not None:
        for name, child in children:
            if name == "value":
                return child
    raise IntrospectionError("Failed to unwrap Arc")


def unwrap_option(option_val):
    """Unwrap ``Option<T>`` and return ``(is_some, value_or_none)``."""
    failed = object()

    def some_payload(active):
        for field_name in ('__0', '0'):
            try:
                return active[field_name]
            except (gdb.error, KeyError):
                continue

        try:
            fields = active.type.fields()
        except (gdb.error, AttributeError):
            fields = []
        if len(fields) == 1:
            try:
                return active[fields[0]]
            except (gdb.error, KeyError):
                pass

        return active

    def from_tagged_layout():
        try:
            fields = option_val.type.fields()
            if not fields:
                return (False, None)

            content = option_val[fields[0]]
            content_fields = content.type.fields()
            if not content_fields:
                return (False, None)

            if len(content_fields) == 1:
                active_field = content_fields[0]
            else:
                active_idx = int(content[content_fields[0]]) + 1
                if active_idx < 1 or active_idx >= len(content_fields):
                    return failed
                active_field = content_fields[active_idx]

            if active_field.name == "None":
                return (False, None)

            return (True, some_payload(content[active_field]))
        except (gdb.error, KeyError, TypeError, ValueError):
            return failed

    def from_direct_variant():
        # Some niche-optimized Options expose only the active variant.
        try:
            for field in option_val.type.fields():
                if field.name == "Some":
                    return (True, some_payload(option_val[field]))
        except (gdb.error, KeyError, TypeError):
            pass
        return failed

    result = from_tagged_layout()
    if result is not failed:
        return result

    result = from_direct_variant()
    if result is not failed:
        return result

    pp = _get_pp(option_val)
    if pp is None:
        raise IntrospectionError("Failed to unwrap Option")

    children = _pp_children(pp)
    if children:
        return (True, children[0][1])
    text = _pp_to_string(pp)
    if text is not None and text.strip() == "None":
        return (False, None)

    raise IntrospectionError("Failed to unwrap Option")


def read_vec(vec_val):
    """Read elements from ``Vec<T>`` via the rust-gdb printer."""
    children = _pp_children(_require_pp(vec_val, "Vec"))
    if children is not None:
        return [child for _name, child in children]
    raise IntrospectionError("Failed to read Vec")


def read_btree_map(btree_map_val):
    """Yield ``(key, value)`` pairs from ``BTreeMap<K, V>``."""
    try:
        children = list(_require_pp(btree_map_val, "BTreeMap").children())
        for idx in range(0, len(children) - 1, 2):
            yield (children[idx][1], children[idx + 1][1])
    except Exception as error:
        raise IntrospectionError(f"Failed to traverse BTreeMap: {error}")


def _usize_max():
    return (1 << (gdb.lookup_type('usize').sizeof * 8)) - 1


def unwrap_weak(weak_val):
    """Dereference ``Weak<T>``; return ``None`` when the target is gone."""
    pp = _get_pp(weak_val)
    if pp is not None:
        children = _pp_children(pp)
        if children is not None:
            for name, child in children:
                if name == "value":
                    return child
            return None
        raise IntrospectionError("Failed to unwrap Weak")

    try:
        ptr = weak_val['ptr']['pointer']
        ptr_int = int(ptr)
        if ptr_int == 0 or ptr_int == _usize_max():
            return None
        arc_inner = ptr.dereference()
        if read_scalar(arc_inner['strong']) == 0:
            return None
        return arc_inner['data']
    except (gdb.error, IntrospectionError) as error:
        raise IntrospectionError(f"Failed to unwrap Weak: {error}")


def read_slot_vec(slot_vec_val):
    """Yield ``(index, value)`` for occupied ``SlotVec<T>`` entries."""
    try:
        slots = read_vec(slot_vec_val['slots'])
        for idx, slot in enumerate(slots):
            is_some, value = unwrap_option(slot)
            if is_some:
                yield (idx, value)
    except (gdb.error, IntrospectionError) as error:
        raise IntrospectionError(f"Failed to read SlotVec: {error}")
