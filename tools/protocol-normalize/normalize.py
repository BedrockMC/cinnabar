"""Pinned, binding-scoped generated-source normalization; no compiler required."""
import argparse
import hashlib
import json
import re
from pathlib import Path

SOURCE = Path("crates/protocol/vendor/valentine/bedrock_versions/v1_26_44/src")
FILES = ("common.rs", "mcpe.rs", "proto.rs", "types.rs", "borrowed.rs")
TOKEN = re.compile(r'//[^\n]*|/\*.*?\*/|"(?:\\.|[^"\\])*"|[A-Za-z_][A-Za-z_0-9]*|.', re.S)


class ShapeError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise ShapeError(message)


def digest(data):
    return hashlib.sha256(data.replace(b"\r\n", b"\n")).hexdigest()


def uniform_source(data):
    text = data.decode("utf-8")
    eol = "\r\n" if "\r\n" in text else "\n"
    require("\r" not in text.replace("\r\n", ""), "unsupported source line ending")
    require(eol == "\n" or "\n" not in text.replace("\r\n", ""), "mixed source line endings")
    return text.replace("\r\n", "\n"), eol


def apply_layout(text, edits, reverse=False):
    """Exact reviewed layout spans, not a general formatter or syntax mask."""
    previous = 0
    shift = 0
    checked = []
    for edit in edits:
        offset, before, after = edit["offset"], edit["before"], edit["after"]
        require(type(offset) is int and offset >= 0, "invalid layout offset")
        require(isinstance(before, str) and isinstance(after, str), "invalid layout span")
        require("\r" not in before + after and bool(before), "invalid layout line ending or empty preimage")
        start = offset + shift if reverse else offset
        expected, replacement = (after, before) if reverse else (before, after)
        require(start >= previous and start + len(expected) <= len(text), "overlapping or out-of-range layout span")
        require(text[start:start + len(expected)] == expected, "layout span preimage drift")
        checked.append((start, start + len(expected), replacement))
        previous = start + len(expected)
        shift += len(after) - len(before)
    for start, end, replacement in reversed(checked):
        text = text[:start] + replacement + text[end:]
    return text


def items(text):
    """Read generated top-level declarations/impls without interpreting literals."""
    tokens = list(TOKEN.finditer(text))
    depth = 0
    start = None
    for token in tokens:
        value = token.group()
        if depth == 0 and value in ("pub", "impl"):
            start = token.start()
        if value == "{":
            if depth == 0:
                header = text[start:token.start()] if start is not None else ""
                body = token.end()
            depth += 1
        elif value == "}":
            depth -= 1
            require(depth >= 0, "unbalanced generated source")
            if depth == 0 and start is not None:
                yield start, token.end(), header, text[body:token.start()]
                start = None
    require(depth == 0, "unclosed generated source")


def declarations(text):
    result = {}
    for start, end, header, body in items(text):
        match = re.fullmatch(r"pub (struct|enum) (\w+)\s*", header)
        if match:
            require(match[2] not in result, "duplicate type binding")
            result[match[2]] = (match[1], body, start, end)
    return result


def fields(declaration):
    require(declaration[0] == "struct", "expected record binding")
    return re.findall(r"(?m)^\s*pub (\w+): ([^\n]+),\s*$", declaration[1])


def target_type(field, declarations_by_name):
    targets = [word for word in re.findall(r"\w+", field[1]) if word in declarations_by_name]
    require(len(targets) == 1, "supporting type selector is ambiguous")
    return targets[0]


def numeric_variant(text, owner, number):
    matches = set(re.findall(rf"\b{re.escape(owner)}::(\w+)\s*=>\s*{number}\s*,", text))
    require(len(matches) == 1, "numeric variant selector is missing or ambiguous")
    return matches.pop()


def discover(sources, manifest):
    owned = {}
    for filename in ("proto.rs", "types.rs"):
        owned.update(declarations(sources[filename]))
    borrowed = declarations(sources["borrowed.rs"])
    types = {}
    scoped = {}

    def rename_type(name, neutral):
        require(name in owned, "selected payload declaration missing")
        require(neutral not in owned or name == neutral, "neutral type collision")
        require(name not in types or types[name] == neutral, "conflicting type selector")
        types[name] = neutral
        if name + "View" in borrowed:
            require(neutral + "View" not in borrowed or name == neutral, "neutral view collision")
            types[name + "View"] = neutral + "View"
            require(len(fields(owned[name])) == len(fields(borrowed[name + "View"]))
                    if owned[name][0] == "struct" else True, "borrowed record shape drift")
        elif owned[name][0] == "struct":
            raise ShapeError("selected record has no borrowed correspondence")
        alias = "Borrowed" + name
        if re.search(rf"pub type {alias} = {name}View;", sources["borrowed.rs"]):
            require(not re.search(rf"pub type Borrowed{neutral}\b", sources["borrowed.rs"])
                    or name == neutral, "neutral borrowed alias collision")
            types[alias] = "Borrowed" + neutral

    def rename_fields(name, indices):
        record = fields(owned[name])
        mapping = scoped.setdefault(name, {})
        for index in indices:
            require(index < len(record), "field ordinal out of range")
            old = record[index][0]
            neutral = f"reserved_field_{index}"
            require(neutral not in dict(record) or old == neutral, "neutral field collision")
            mapping[old] = neutral
        view = name + "View"
        if view in borrowed:
            view_fields = fields(borrowed[view])
            require([entry[0] for entry in view_fields] == [entry[0] for entry in record],
                    "borrowed field correspondence drift")
            scoped[view] = dict(mapping)

    def support(name, neutral):
        rename_type(name, neutral)
        if owned[name][0] == "struct":
            rename_fields(name, range(len(fields(owned[name]))))
        else:
            # Enum alternatives are discovered by their numeric codec arms.
            alternatives = re.findall(r"(?m)^\s*(\w+)(?:\([^\n]*\))?,\s*$", owned[name][1])
            mapping = {}
            for alternative in alternatives:
                if alternative == "Unknown":
                    continue
                numbers = set(re.findall(rf"\b{name}::{alternative}\s*=>\s*(-?\d+)\s*,", sources["types.rs"]))
                require(len(numbers) == 1, "supporting enum numeric shape drift")
                mapping[alternative] = "Reserved" + next(iter(numbers)).replace("-", "Minus")
            scoped[name] = mapping

    packet_names = {}
    for number in manifest["packet_ids"]:
        matches = re.findall(rf"(?m)^\s*(\w+) = {number}u32,", sources["common.rs"])
        require(len(matches) == 1, "packet numeric selector drift")
        name = matches[0]
        packet_names[number] = name
        rename_type(name, f"ReservedPacket{number}")
        rename_fields(name, range(len(fields(owned[name]))))

    for selector in manifest["supporting_records"]:
        name = packet_names[selector["packet"]] if "packet" in selector else selector["owner"]
        for ordinal in selector["path"]:
            record = fields(owned[name])
            require(ordinal < len(record), "support path ordinal drift")
            name = target_type(record[ordinal], owned)
        support(name, selector["neutral"])

    for owner, ordinals in manifest["field_ordinals"].items():
        rename_fields(owner, ordinals)

    for owner, numbers in manifest["enum_values"].items():
        for number in numbers:
            alternative = numeric_variant(sources["types.rs"], owner, number)
            scoped.setdefault(owner, {})[alternative] = f"Reserved{number}"

    # The action payload is selected from both unions by its codec discriminator.
    payloads = set()
    for owner in manifest["action_unions"]:
        declaration = owned[owner]
        alternatives = re.findall(r"(?m)^\s*(\w+)\((\w+)\),\s*$", declaration[1])
        candidates = []
        for alternative, payload in alternatives:
            discriminator = manifest["action_union_discriminator"]
            pattern = rf"{owner}::{alternative}\(\s*value\s*,?\s*\) => \{{\s*crate::bedrock::codec::BedrockSized::encoded_size\(\s*&?crate::bedrock::codec::VarUInt\(\s*{discriminator} as u32\s*,?\s*\)"
            if re.search(pattern, sources["types.rs"]):
                candidates.append((alternative, payload))
        require(len(candidates) == 1, "action union discriminator shape drift")
        alternative, payload = candidates[0]
        payloads.add(payload)
        scoped.setdefault(owner, {})[alternative] = "Reserved9"
        require(owner + "View" not in borrowed, "unexpected separate borrowed action union")
    require(len(payloads) == 1, "action unions disagree on payload")
    payload = payloads.pop()
    require(fields(owned[payload]) == [("actiontype", "EnumsItemStackRequestActionType")]
            or fields(owned[payload]) == [("reserved_field_0", "EnumsItemStackRequestActionType")],
            "action payload field shape drift")
    support(payload, "ReservedStackRequestAction9")

    for selector in manifest["reserved_unions"]:
        owner = selector["owner"]
        candidates = []
        for alternative, payload in re.findall(r"(?m)^\s*(\w+)\((\w+)\),\s*$", owned[owner][1]):
            number = selector["discriminator"]
            pattern = rf"{owner}::{alternative}\(\s*value\s*,?\s*\) => \{{\s*crate::bedrock::codec::BedrockSized::encoded_size\(\s*&?crate::bedrock::codec::VarUInt\(\s*{number} as u32\s*,?\s*\)"
            if re.search(pattern, sources["types.rs"]):
                candidates.append((alternative, payload))
        require(len(candidates) == 1, "reserved union discriminator drift")
        alternative, payload = candidates[0]
        require([field[1] for field in fields(owned[payload])] == selector["field_types"],
                "reserved union payload field shape drift")
        scoped.setdefault(owner, {})[alternative] = "Reserved" + str(number)
        support(payload, selector["neutral"])

    # Supporting declarations may not gain an unreviewed incoming field edge.
    selected = set(types)
    allowed_edges = set(tuple(edge) for edge in manifest["shared_incoming_fields"])
    for owner, declaration in owned.items():
        if declaration[0] != "struct" or owner in selected:
            continue
        for index, field in enumerate(fields(declaration)):
            referenced = set(re.findall(r"\w+", field[1])) & selected
            if referenced:
                require((owner, index) in allowed_edges, "unreviewed shared payload reference")
    allowed_unions = set(manifest["action_unions"]) | {item["owner"] for item in manifest["reserved_unions"]}
    for owner, declaration in owned.items():
        if declaration[0] == "enum" and owner not in selected:
            references = set(re.findall(r"\w+", declaration[1])) & selected
            require(not references or owner in allowed_unions, "unreviewed enum payload reference")
    return types, scoped


def rewrite_tokens(text, mapping):
    return TOKEN.sub(lambda match: mapping.get(match.group(), match.group()), text)


def rewrite_source(text, types, scoped):
    edits = []
    for start, end, header, body in items(text):
        owners = set(re.findall(r"\w+", header)) & set(scoped)
        if not owners:
            continue
        mapping = {}
        for owner in owners:
            for old, neutral in scoped[owner].items():
                require(old not in mapping or mapping[old] == neutral, "scoped binding collision")
                mapping[old] = neutral
        item = text[start:end]
        if header.lstrip().startswith("pub enum") or any(
                owner in scoped and re.search(rf"pub enum {owner}\b", header) for owner in owners):
            item = rewrite_tokens(item, mapping)
        elif header.lstrip().startswith("pub struct") or any(
                name.startswith("reserved_field_") for pair in mapping.items() for name in pair):
            item = rewrite_tokens(item, mapping)
        else:
            for owner in owners:
                for old, neutral in scoped[owner].items():
                    item = re.sub(rf"\b({owner}|Self)::{old}\b", lambda match: match[1] + "::" + neutral, item)
        edits.append((start, end, item))
    for start, end, replacement in reversed(edits):
        text = text[:start] + replacement + text[end:]
    text = rewrite_tokens(text, types)
    # Only exact debug type labels are eligible string-token changes.
    for old, neutral in types.items():
        text = re.sub(rf'(debug_(?:struct|tuple)\(")({old})("\))',
                      lambda match: match[1] + neutral + match[3], text)
    return text


def normalize(data, manifest, verify_hashes=True):
    require(set(data) == set(FILES), "source file set drift")
    decoded = {name: uniform_source(value) for name, value in data.items()}
    hashes = {name: digest(value) for name, value in data.items()}
    if verify_hashes and hashes == manifest.get("normalized_sha256"):
        unformatted = {name: apply_layout(value[0], manifest["layout_edits"][name], reverse=True)
                       for name, value in decoded.items()}
        require({name: digest(value.encode()) for name, value in unformatted.items()} == manifest["preformat_sha256"],
                "canonical inverse layout fingerprint drift")
        require(all(apply_layout(value, manifest["layout_edits"][name]) == decoded[name][0]
                    for name, value in unformatted.items()), "canonical layout round-trip drift")
        return dict(data)
    if verify_hashes:
        require(hashes == manifest["input_sha256"], "pinned source drift or mixed normalization state")
    sources = {name: value[0] for name, value in decoded.items()}
    types, scoped = discover(sources, manifest)
    renamed = {name: rewrite_source(text, types, scoped) for name, text in sources.items()}
    if verify_hashes:
        require({name: digest(value.encode()) for name, value in renamed.items()} == manifest["preformat_sha256"],
                "preformat source fingerprint drift")
    formatted = {name: apply_layout(text, manifest["layout_edits"][name]) for name, text in renamed.items()}
    output = {name: text.replace("\n", decoded[name][1]).encode() for name, text in formatted.items()}
    inverse_types = {new: old for old, new in types.items()}
    require(len(inverse_types) == len(types), "type rename is not one-to-one")
    inverse_scoped = {
        types.get(owner, owner): {new: old for old, new in mapping.items()}
        for owner, mapping in scoped.items()
    }
    for name, value in output.items():
        unformatted = apply_layout(formatted[name], manifest["layout_edits"][name], reverse=True)
        require(unformatted == renamed[name], "layout inverse byte drift")
        restored = rewrite_source(unformatted, inverse_types, inverse_scoped)
        require(restored.replace("\n", decoded[name][1]).encode() == data[name],
                "unapproved token, debug-span or layout mutation")
        literals = lambda text: [match.group() for match in TOKEN.finditer(text)
                                 if match.group().startswith('"') or match.group()[0].isdigit()]
        require(literals(sources[name]) == literals(value.decode("utf-8")),
                "wire numeric or error-string literal changed")
    # Discover again using the same numeric/ordinal selectors. No old aliases
    # are persisted; all canonical replacements must now be identity mappings.
    canonical = formatted
    final_types, final_scoped = discover(canonical, manifest)
    require(all(old == new for old, new in final_types.items()), "noncanonical type binding remains")
    require(all(old == new for mapping in final_scoped.values() for old, new in mapping.items()),
            "noncanonical scoped binding remains")
    if verify_hashes:
        require({name: digest(value) for name, value in output.items()} == manifest["normalized_sha256"],
                "normalized source fingerprint drift")
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--patch", action="store_true", help="emit an apply_patch patch; never write source")
    args = parser.parse_args()
    manifest = json.loads(Path(__file__).with_name("manifest.json").read_text())
    data = {name: (args.root / SOURCE / name).read_bytes() for name in FILES}
    output = normalize(data, manifest)
    if output == data:
        print("Generated reservations are canonical.")
        return
    require(args.patch, "generated reservations need normalization; use --patch")
    import difflib
    print("*** Begin Patch")
    for name in FILES:
        if data[name] == output[name]:
            continue
        print(f"*** Update File: {args.root / SOURCE / name}")
        diff = list(difflib.unified_diff(data[name].decode().splitlines(), output[name].decode().splitlines(), n=3))
        for line in diff[2:]:
            print("@@" if line.startswith("@@") else line)
    print("*** End Patch")


if __name__ == "__main__":
    main()
