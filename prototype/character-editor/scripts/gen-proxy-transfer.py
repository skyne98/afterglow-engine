#!/usr/bin/env python3
"""Offline morph-transfer onto a genital proxy topology.

Produces a SELF-CONTAINED skinned glTF where the proxy (which carries genitals)
also owns the body-morph library as native morph targets. The runtime editor
blends morphTargetInfluences directly - NO refit at runtime.

How it works:
  1. create_human (base) + game rig.
  2. apply the genital proxy as a covering mesh - same skeleton.
  3. for each base morph target, set it on the base, REFIT the proxy, capture
     the proxy vertex positions, and store them as a proxy-native morph target
     (delta = proxy_at(morph=1) - proxy_at(morph=0)).
  4. export ONLY the proxy + rig: one mesh, genitals + all morphs, skinned.

Env: SEX=male|female, CHAR_OUT=path.glb, FACE_TARGET_ROOT=pack directory,
     HAIR_ROOT=directory with short04 and ponytail01 MHCLO assets,
     MORPHS=(optional comma list of base-target names to transfer;
     default = all canonical controls plus all non-empty face targets)
"""
import importlib, sys, os, bpy, json, glob, re
from array import array
from mathutils import Vector
from mathutils.bvhtree import BVHTree

def dyn(pkg, key):
    for m in sys.modules:
        if m.endswith(pkg): return getattr(importlib.import_module(m), key)
    raise ValueError("no " + pkg)

import addon_utils
addon_utils.enable('bl_ext.user_default.mpfb', default_set=True)

HS = dyn("mpfb.services.humanservice", "HumanService")
CS = dyn("mpfb.services.clothesservice", "ClothesService")
TS = dyn("mpfb.services.targetservice", "TargetService")
LS = dyn("mpfb.services.locationservice", "LocationService")
GOP = dyn("mpfb.entities.objectproperties", "GeneralObjectProperties")
HP = dyn("mpfb.entities.objectproperties", "HumanObjectProperties")
Mhclo = dyn("mpfb.entities.clothes.mhclo", "Mhclo")

SEX = os.environ.get("SEX", "male")
OUT = os.environ.get("CHAR_OUT", "/tmp/char.glb")
PROXY_ROOT = os.environ.get("PROXY_ROOT", "")
FACE_TARGET_ROOT = os.environ.get("FACE_TARGET_ROOT", "")
HAIR_ROOT = os.environ.get("HAIR_ROOT", "")
HAIR_STYLES = (
    ("short04", "Short 04"),
    ("ponytail01", "Ponytail 01"),
)

def split_hair_for_runtime(hair):
    """Give each card face its own vertices so glTF keeps one MHCLO map per output vertex."""
    source_mesh = hair.data
    source_uv = source_mesh.uv_layers.active
    vertices = []
    faces = []
    uvs = []
    source_by_vertex = []
    source_group_names = [group.name for group in hair.vertex_groups]
    source_weights = [
        [(membership.group, membership.weight) for membership in vertex.groups]
        for vertex in source_mesh.vertices
    ]
    for polygon in source_mesh.polygons:
        face = []
        for loop_index in polygon.loop_indices:
            source = source_mesh.loops[loop_index].vertex_index
            face.append(len(vertices))
            vertices.append(tuple(source_mesh.vertices[source].co))
            source_by_vertex.append(source)
            uvs.append(tuple(source_uv.data[loop_index].uv) if source_uv else (0.0, 0.0))
        faces.append(face)
    runtime_mesh = bpy.data.meshes.new(source_mesh.name + "-runtime")
    runtime_mesh.from_pydata(vertices, [], faces)
    runtime_mesh.update()
    runtime_uv = runtime_mesh.uv_layers.new(name="UVMap")
    for loop_index, uv in enumerate(uvs):
        runtime_uv.data[loop_index].uv = uv
    hair.data = runtime_mesh
    runtime_groups = [hair.vertex_groups.new(name=name) for name in source_group_names]
    for vertex, source in enumerate(source_by_vertex):
        for group, weight in source_weights[source]:
            runtime_groups[group].add([vertex], weight, 'REPLACE')
    bpy.data.meshes.remove(source_mesh)
    for polygon in runtime_mesh.polygons:
        polygon.use_smooth = True
    runtime_mesh.update()
    return source_by_vertex

# --- build character + proxy ---------------------------------------------
b = HS.create_human()
# Bake the sex macro (and a default caucasian ethnicity) into the BASE so the
# refit proxy conforms to a genuinely male vs female silhouette (not a shared
# unisex envelope). gender=1 -> male, 0 -> female (MPFB macro semantics).
HP.set_value("gender", 1.0 if SEX == "male" else 0.0, entity_reference=b)
HP.set_value("caucasian", 1.0, entity_reference=b)
HP.set_value("asian", 0.0, entity_reference=b)
HP.set_value("african", 0.0, entity_reference=b)
TS.reapply_macro_details(b)
rig = HS.add_builtin_rig(b, "game_engine")

proxy_file = glob.glob(os.path.join(PROXY_ROOT, "*.proxy"))[0] if PROXY_ROOT else ""
if not proxy_file:
    raise SystemExit("PROXY_ROOT must point at a directory with a .proxy file")
mhclo = Mhclo(); mhclo.load(proxy_file)
proxy = mhclo.load_mesh(bpy.context)
GOP.set_value("object_type", "Proxymeshes", entity_reference=proxy)
GOP.set_value("scale_factor", GOP.get_value("scale_factor", entity_reference=b), entity_reference=proxy)
CS.fit_clothes_to_human(proxy, b, mhclo)
mhclo.set_scalings(bpy.context, b)
CS.set_up_rigging(b, proxy, rig, mhclo, interpolate_weights=True, import_subrig=False, import_weights=True)

if not HAIR_ROOT:
    raise SystemExit("HAIR_ROOT must point at extracted MakeHuman hair assets")
hair_assets = []
for style_id, style_label in HAIR_STYLES:
    hair_file = os.path.join(HAIR_ROOT, style_id, style_id + ".mhclo")
    if not os.path.isfile(hair_file):
        raise RuntimeError(f"missing hair asset {hair_file}")
    hair_mhclo = Mhclo()
    hair_mhclo.load(hair_file)
    hair = hair_mhclo.load_mesh(bpy.context)
    hair.name = f"Hair-{style_id}"
    hair.data.name = f"Hair-{style_id}"
    GOP.set_value("object_type", "Proxymeshes", entity_reference=hair)
    GOP.set_value("scale_factor", GOP.get_value("scale_factor", entity_reference=b), entity_reference=hair)
    CS.fit_clothes_to_human(hair, b, hair_mhclo)
    hair_mhclo.set_scalings(bpy.context, b)
    CS.set_up_rigging(
        b, hair, rig, hair_mhclo,
        interpolate_weights=True, import_subrig=False, import_weights=True,
    )
    binding_sources = split_hair_for_runtime(hair)
    hair.name = f"Hair-{style_id}"
    hair.data.name = f"Hair-{style_id}"
    hair_assets.append((style_id, style_label, hair, hair_mhclo, binding_sources))

# Compose the two authored SurfaceWraps. Hair vertices refer to the hm08 base,
# while PunkElvs is another hm08 wrap. Find each hair anchor on the proxy's
# offset-free anchor surface, then add the proxy displacement at that exact
# point. This preserves the authored hair offset without an arbitrary gap.
def mixed_base_coords():
    key = b.shape_key_add(name="temporary_full_base_capture", from_mix=True)
    coords = [key.data[index].co.copy() for index in range(len(b.data.vertices))]
    b.shape_key_remove(key)
    return coords

def binding_anchor(binding, base_coords):
    result = Vector()
    for source, weight in zip(binding["verts"], binding["weights"]):
        result += base_coords[source] * weight
    return result

def triangle_weights(point, first, second, third):
    edge0 = second - first
    edge1 = third - first
    relative = point - first
    dot00 = edge0.dot(edge0)
    dot01 = edge0.dot(edge1)
    dot11 = edge1.dot(edge1)
    dot20 = relative.dot(edge0)
    dot21 = relative.dot(edge1)
    denominator = dot00 * dot11 - dot01 * dot01
    if abs(denominator) < 1.0e-12:
        raise RuntimeError("the proxy anchor surface has a degenerate triangle")
    second_weight = (dot11 * dot20 - dot01 * dot21) / denominator
    third_weight = (dot00 * dot21 - dot01 * dot20) / denominator
    return (1.0 - second_weight - third_weight, second_weight, third_weight)

def prepare_composed_hair():
    base_coords = mixed_base_coords()
    proxy_anchors = [binding_anchor(mhclo.verts[index], base_coords) for index in range(len(proxy.data.vertices))]
    proxy_faces = [
        (polygon.vertices[0], polygon.vertices[index], polygon.vertices[index + 1])
        for polygon in proxy.data.polygons
        for index in range(1, len(polygon.vertices) - 1)
    ]
    anchor_tree = BVHTree.FromPolygons(proxy_anchors, proxy_faces, all_triangles=True)
    support_sources = set()
    prepared = {}
    for style_id, _, hair, hair_mhclo, binding_sources in hair_assets:
        style_bindings = []
        hair.vertex_groups.clear()
        hair_groups = {
            group.index: hair.vertex_groups.new(name=group.name)
            for group in proxy.vertex_groups
        }
        for vertex, binding_source in enumerate(binding_sources):
            hair_binding = hair_mhclo.verts[binding_source]
            hair_anchor = binding_anchor(hair_binding, base_coords)
            nearest, _, face_index, _ = anchor_tree.find_nearest(hair_anchor)
            if nearest is None or face_index is None:
                raise RuntimeError(f"{style_id} has no proxy anchor")
            face = proxy_faces[face_index]
            weights = triangle_weights(nearest, *(proxy_anchors[index] for index in face))
            proxy_anchor = sum(
                (proxy_anchors[parent] * weight for parent, weight in zip(face, weights)),
                Vector(),
            )
            proxy_position = sum(
                (proxy.data.vertices[parent].co * weight for parent, weight in zip(face, weights)),
                Vector(),
            )
            hair.data.vertices[vertex].co += proxy_position - proxy_anchor
            support_sources.update(face)
            style_bindings.append((face, weights))

            bone_weights = {}
            for parent, surface_weight in zip(face, weights):
                for membership in proxy.data.vertices[parent].groups:
                    bone_weights[membership.group] = (
                        bone_weights.get(membership.group, 0.0)
                        + surface_weight * membership.weight
                    )
            total = sum(max(weight, 0.0) for weight in bone_weights.values())
            if total <= 1.0e-8:
                raise RuntimeError(f"{style_id} vertex {vertex} has no proxy rig weights")
            for group, weight in bone_weights.items():
                if weight > 0.0:
                    hair_groups[group].add([vertex], weight / total, 'REPLACE')
        hair.data.update()
        prepared[style_id] = style_bindings
    return sorted(support_sources), prepared

proxy_support_sources, prepared_hair_bindings = prepare_composed_hair()

def refit():
    bpy.context.view_layer.update()
    CS.fit_clothes_to_human(proxy, b, mhclo)
    bpy.context.view_layer.update()

def proxy_coords():
    coords = array('f', [0.0]) * (len(proxy.data.vertices) * 3)
    proxy.data.vertices.foreach_get("co", coords)
    return coords


def max_displacement(a, b):
    maximum = 0.0
    for offset in range(0, len(a), 3):
        dx = a[offset] - b[offset]
        dy = a[offset + 1] - b[offset + 1]
        dz = a[offset + 2] - b[offset + 2]
        distance = (dx * dx + dy * dy + dz * dz) ** 0.5
        maximum = max(maximum, distance)
    return maximum

# Cook one compact hm08 driver shared by both selected styles. The MHCLO parser
# has already converted source offsets to Blender coordinates.
driver_sources = set()
for source in proxy_support_sources:
    driver_sources.update(mhclo.verts[source]["verts"])
for scale in (mhclo.x_scale, mhclo.y_scale, mhclo.z_scale):
    if not scale:
        raise RuntimeError("proxy MHCLO data has no axis scale")
    driver_sources.update(scale[:2])
for _, _, _, hair_mhclo, _ in hair_assets:
    for scale in (hair_mhclo.x_scale, hair_mhclo.y_scale, hair_mhclo.z_scale):
        if not scale:
            raise RuntimeError("hair MHCLO data has no axis scale")
        driver_sources.update(scale[:2])
driver_sources = sorted(driver_sources)
driver_local = {source: local for local, source in enumerate(driver_sources)}

def mixed_driver_coords():
    key = b.shape_key_add(name="temporary_hair_driver_capture", from_mix=True)
    coords = array('f', [0.0]) * (len(driver_sources) * 3)
    for local, source in enumerate(driver_sources):
        co = key.data[source].co
        coords[local * 3:local * 3 + 3] = array('f', (co.x, co.y, co.z))
    b.shape_key_remove(key)
    return coords

def effective_scale_record(hair_mhclo):
    # MPFB applies x_scale to Blender X, z_scale to Blender Y, and y_scale to
    # Blender Z after it converts the MHCLO source coordinate system.
    return [
        [driver_local[hair_mhclo.x_scale[0]], driver_local[hair_mhclo.x_scale[1]], hair_mhclo.x_scale[2], 0],
        [driver_local[hair_mhclo.z_scale[0]], driver_local[hair_mhclo.z_scale[1]], hair_mhclo.z_scale[2], 1],
        [driver_local[hair_mhclo.y_scale[0]], driver_local[hair_mhclo.y_scale[1]], hair_mhclo.y_scale[2], 2],
    ]

def effective_scales(driver, records):
    values = []
    for first, second, source_distance, axis in records:
        distance = abs(driver[first * 3 + axis] - driver[second * 3 + axis])
        values.append(distance / source_distance)
    return values

def cook_hair_style(style_id, label, hair, hair_mhclo, binding_sources, neutral_driver):
    if len(binding_sources) != len(hair.data.vertices):
        raise RuntimeError(f"{style_id} runtime binding count is incorrect")
    scales = effective_scale_record(hair_mhclo)
    scale_values = effective_scales(neutral_driver, scales)
    parents = []
    weights = []
    offsets = []
    maximum_error = 0.0
    for vertex in range(len(hair.data.vertices)):
        binding = hair_mhclo.verts[binding_sources[vertex]]
        local_parents = [driver_local[source] for source in binding["verts"]]
        binding_weights = list(binding["weights"])
        binding_offsets = list(binding["offsets"])
        parents.extend(local_parents)
        weights.extend(binding_weights)
        offsets.extend(binding_offsets)
        fitted = [binding_offsets[axis] * scale_values[axis] for axis in range(3)]
        for parent in range(3):
            source_offset = local_parents[parent] * 3
            for axis in range(3):
                fitted[axis] += neutral_driver[source_offset + axis] * binding_weights[parent]
        actual = hair.data.vertices[vertex].co
        maximum_error = max(
            maximum_error,
            ((actual.x - fitted[0]) ** 2 + (actual.y - fitted[1]) ** 2 + (actual.z - fitted[2]) ** 2) ** 0.5,
        )
    if maximum_error > 3.0e-6:
        raise RuntimeError(f"{style_id} cooked fit differs from MPFB by {maximum_error}")
    return {
        "id": style_id,
        "label": label,
        "mesh": hair.name,
        "vertexCount": len(hair.data.vertices),
        "parents": parents,
        "weights": weights,
        "offsets": offsets,
        "scales": scales,
        "neutralMaximumError": maximum_error,
    }

def cook_composed_style(style_id, label, hair, hair_mhclo, bindings, neutral_driver, support_local):
    scales = effective_scale_record(hair_mhclo)
    scale_values = effective_scales(neutral_driver, scales)
    parents = []
    weights = []
    offsets = []
    maximum_error = 0.0
    for vertex, (face, binding_weights) in enumerate(bindings):
        local_parents = [support_local[source] for source in face]
        surface = Vector()
        for parent, weight in zip(face, binding_weights):
            surface += proxy.data.vertices[parent].co * weight
        actual = hair.data.vertices[vertex].co
        offset = actual - surface
        normalized_offset = [offset[axis] / scale_values[axis] for axis in range(3)]
        parents.extend(local_parents)
        weights.extend(binding_weights)
        offsets.extend(normalized_offset)
        fitted = [normalized_offset[axis] * scale_values[axis] for axis in range(3)]
        for parent, weight in zip(local_parents, binding_weights):
            source_offset = parent * 3
            for axis in range(3):
                fitted[axis] += proxy_support_neutral[source_offset + axis] * weight
        maximum_error = max(
            maximum_error,
            ((actual.x - fitted[0]) ** 2 + (actual.y - fitted[1]) ** 2 + (actual.z - fitted[2]) ** 2) ** 0.5,
        )
    if maximum_error > 3.0e-6:
        raise RuntimeError(f"{style_id} composed fit error {maximum_error}")
    return {
        "id": style_id,
        "label": label,
        "mesh": hair.name,
        "vertexCount": len(hair.data.vertices),
        "parents": parents,
        "weights": weights,
        "offsets": offsets,
        "scales": scales,
        "neutralMaximumError": maximum_error,
    }

# Keep the base-mesh eyes, teeth, and tongue. The PunkElvs body proxy does not
# contain the tongue, so its tongueOut face unit has no proxy displacement.
face_helper_groups = {
    "helper-l-eye", "helper-r-eye", "helper-lower-teeth",
    "helper-upper-teeth", "helper-tongue",
}
face_helper_group_indices = {
    b.vertex_groups[name].index for name in face_helper_groups
}
face_helper_candidates = {
    vertex.index for vertex in b.data.vertices
    if any(group.group in face_helper_group_indices for group in vertex.groups)
}
face_helper_source_faces = [
    tuple(polygon.vertices) for polygon in b.data.polygons
    if all(vertex in face_helper_candidates for vertex in polygon.vertices)
]
face_helper_source_vertices = sorted({
    vertex for face in face_helper_source_faces for vertex in face
})
face_helper_local_by_source = {
    source: local for local, source in enumerate(face_helper_source_vertices)
}
face_helper_faces = [
    tuple(face_helper_local_by_source[vertex] for vertex in face)
    for face in face_helper_source_faces
]
if not face_helper_source_vertices or not face_helper_faces:
    raise RuntimeError("the base-mesh face helper geometry is empty")

def face_helper_coords():
    key = b.shape_key_add(name="temporary_face_helper_capture", from_mix=True)
    coords = array('f', [0.0]) * (len(face_helper_source_vertices) * 3)
    for local, source in enumerate(face_helper_source_vertices):
        co = key.data[source].co
        coords[local * 3:local * 3 + 3] = array('f', (co.x, co.y, co.z))
    b.shape_key_remove(key)
    return coords

# --- choose all canonical direct controls ---------------------------------
targets_root = LS.get_mpfb_data("targets")
morph_list = os.environ.get("MORPHS", "").strip()
control_specs = []
files_by_name = {}

def target_file(category, name):
    hits = glob.glob(os.path.join(targets_root, category, name + ".target*"))
    if not hits:
        raise RuntimeError(f"no target file for {category}/{name}")
    return hits[0]

if morph_list:
    for name in morph_list.split(","):
        hits = glob.glob(os.path.join(targets_root, "*", name + ".target*"))
        if not hits and FACE_TARGET_ROOT:
            hits = glob.glob(os.path.join(FACE_TARGET_ROOT, "targets", "*", name + ".target*"))
        if not hits:
            raise RuntimeError(f"no target file for {name}")
        files_by_name[name] = hits[0]
        control_specs.append({"category": "custom", "label": name,
                              "negative": "", "positive": name})
else:
    with open(os.path.join(targets_root, "target.json")) as manifest_file:
        target_manifest = json.load(manifest_file)
    for category, category_data in target_manifest.items():
        if category == "genitals" and SEX != "male":
            continue
        for control in category_data.get("categories", []):
            for name in control.get("targets", []):
                files_by_name.setdefault(name, target_file(category, name))
            opposites = control.get("opposites")
            if not opposites:
                control_specs.append({
                    "category": category,
                    "label": control["label"],
                    "negative": "",
                    "positive": control["targets"][0],
                })
            elif control["has_left_and_right"]:
                for side in ("left", "right"):
                    control_specs.append({
                        "category": category,
                        "label": f"{control['label']} ({side})",
                        "negative": opposites[f"negative-{side}"],
                        "positive": opposites[f"positive-{side}"],
                    })
            else:
                control_specs.append({
                    "category": category,
                    "label": control["label"],
                    "negative": opposites["negative-unsided"],
                    "positive": opposites["positive-unsided"],
                })

    # MPFB supplies these direct targets outside target.json.
    for path in sorted(glob.glob(os.path.join(targets_root, "asym", "*.target*"))):
        name = os.path.basename(path).replace(".target.gz", "").replace(".target", "")
        files_by_name.setdefault(name, path)
        control_specs.append({"category": "asymmetry", "label": name,
                              "negative": "", "positive": name})

    # The official CC0 functional packs are source assets in this repository.
    # Silence files contain no displacement, so they are labels and not morphs.
    if not FACE_TARGET_ROOT:
        raise RuntimeError("FACE_TARGET_ROOT must point at the functional face packs")
    pack_specs = [
        ("faceunits01", "faceunits", "expression"),
        ("visemes01", "visemes", "speech-microsoft"),
        ("visemes02", "visemes", "speech-meta"),
    ]
    face_target_count = 0
    for pack_name, target_directory, base_category in pack_specs:
        manifest_path = os.path.join(FACE_TARGET_ROOT, "packs", pack_name + ".json")
        with open(manifest_path) as manifest_file:
            pack_manifest = json.load(manifest_file)
        for name in sorted(pack_manifest):
            metadata = pack_manifest[name]
            if metadata.get("license") != "CC0":
                raise RuntimeError(f"{pack_name}/{name} is not CC0")
            path = os.path.join(FACE_TARGET_ROOT, "targets", target_directory, name + ".target")
            if not os.path.isfile(path):
                raise RuntimeError(f"missing face target {path}")
            if os.path.getsize(path) == 0:
                print(f"> skipping zero-displacement label {pack_name}/{name}")
                continue
            if name in files_by_name:
                raise RuntimeError(f"duplicate target name {name}")
            category = base_category
            if pack_name == "faceunits01":
                region_match = re.match(r"(brow|cheek|eye|jaw|mouth|nose|tongue)", name)
                if not region_match:
                    raise RuntimeError(f"unknown face-unit region for {name}")
                category = f"expression-{region_match.group(1)}"
                label = re.sub(r"(?<!^)([A-Z])", r" \1", name).capitalize()
            elif pack_name == "visemes01":
                label = name.rsplit("_", 1)[0].replace("_", " / ").upper()
            else:
                label = name.removeprefix("viseme_") + " (Meta)"
            files_by_name[name] = path
            control_specs.append({"category": category, "label": label,
                                  "negative": "", "positive": name})
            face_target_count += 1
    if face_target_count != 87:
        raise RuntimeError(f"expected 87 non-empty face targets, got {face_target_count}")

files = list(files_by_name.values())
structural_target_names = set()
for spec in control_specs:
    if spec["category"].startswith("expression-") or spec["category"].startswith("speech-"):
        continue
    structural_target_names.update(name for name in (spec["negative"], spec["positive"]) if name)

print(f"> {SEX}: transferring {len(files)} morphs onto proxy ({len(proxy.data.vertices)} verts)")

# Capture all fits before shape-key creation. A refit changes the active shape
# key when shape keys exist. Thus, an interleaved refit erases the new target.
refit()
P0 = proxy_coords()
H0 = face_helper_coords()
D0 = mixed_driver_coords()
proxy_support_local = {
    source: local for local, source in enumerate(proxy_support_sources)
}
proxy_support_neutral = array('f')
for source in proxy_support_sources:
    offset = source * 3
    proxy_support_neutral.extend(P0[offset:offset + 3])
proxy_support_mesh = bpy.data.meshes.new("Hair-proxy-support")
proxy_support_mesh.from_pydata(
    [tuple(proxy_support_neutral[offset:offset + 3]) for offset in range(0, len(proxy_support_neutral), 3)],
    [],
    [],
)
proxy_support = bpy.data.objects.new("Hair-proxy-support", proxy_support_mesh)
bpy.context.collection.objects.link(proxy_support)
proxy_support_record = cook_hair_style(
    "proxy", "Proxy support", proxy_support, mhclo, proxy_support_sources, D0,
)
hair_style_records = [
    cook_composed_style(
        style_id,
        label,
        hair,
        hair_mhclo,
        prepared_hair_bindings[style_id],
        D0,
        proxy_support_local,
    )
    for style_id, label, hair, hair_mhclo, _ in hair_assets
]
driver_targets = {}

def record_driver_target(name, positions):
    deltas = []
    for local in range(len(driver_sources)):
        offset = local * 3
        dx = positions[offset] - D0[offset]
        dy = positions[offset + 1] - D0[offset + 1]
        dz = positions[offset + 2] - D0[offset + 2]
        if max(abs(dx), abs(dy), abs(dz)) <= 1.0e-8:
            continue
        deltas.extend((local, dx, dy, dz))
    if deltas:
        if name in driver_targets:
            raise RuntimeError(f"duplicate hair driver target {name}")
        driver_targets[name] = deltas

captures = []
proxynames = []
for i, fpath in enumerate(files, start=1):
    name = os.path.basename(fpath).replace(".target.gz", "").replace(".target", "")
    sk = TS.load_target(b, fpath, weight=0.0)
    sk.value = 1.0
    refit()
    P1 = proxy_coords()
    H1 = face_helper_coords()
    if name in structural_target_names:
        record_driver_target(name, mixed_driver_coords())
    displacement = max(max_displacement(P0, P1), max_displacement(H0, H1))
    if displacement <= 1.0e-6:
        raise RuntimeError(f"morph {name} has no character displacement ({displacement})")
    captures.append((P1, H1))
    proxynames.append(name)
    sk.value = 0.0
    try:
        b.shape_key_remove(sk)
    except Exception:
        pass
    refit()
    if i % 25 == 0:
        print(f">   {i}/{len(files)} morphs captured")

# Capture each two-sided macro from its 0.5 baseline. Cup size changes breast
# volume, and firmness makes that volume rounder. MPFB marks both as female-only.
macro_properties = [
    ("age", "Age"),
    ("muscle", "Muscularity"),
    ("weight", "Weight"),
    ("proportions", "Proportions"),
    ("height", "Height"),
]
if SEX == "female":
    macro_properties += [("cupsize", "Cup size"), ("firmness", "Breast firmness")]

for property_name, label in macro_properties:
    macro_names = {}
    for direction, value in (("decr", 0.0), ("incr", 1.0)):
        target_name = f"macro-{property_name}-{direction}"
        HP.set_value(property_name, value, entity_reference=b)
        TS.reapply_macro_details(b)
        refit()
        P1 = proxy_coords()
        H1 = face_helper_coords()
        record_driver_target(target_name, mixed_driver_coords())
        displacement = max(max_displacement(P0, P1), max_displacement(H0, H1))
        if displacement <= 1.0e-6:
            raise RuntimeError(f"macro {property_name} has no character displacement ({displacement})")
        captures.append((P1, H1))
        proxynames.append(target_name)
        macro_names[direction] = target_name
        HP.set_value(property_name, 0.5, entity_reference=b)
        TS.reapply_macro_details(b)
        refit()
    control_specs.append({"category": "macro", "label": label,
                          "negative": macro_names["decr"],
                          "positive": macro_names["incr"]})

# Ethnicity macros replace the Caucasian macro. A direct target load would add
# two complete ethnicity shapes together and cause invalid body deformation.
def set_ethnicity(race):
    for name in ("caucasian", "asian", "african"):
        HP.set_value(name, 1.0 if name == race else 0.0, entity_reference=b)
    TS.reapply_macro_details(b)
    refit()

for race in ("asian", "african"):
    set_ethnicity(race)
    P1 = proxy_coords()
    H1 = face_helper_coords()
    record_driver_target(f"{race}-{SEX}-young", mixed_driver_coords())
    displacement = max(max_displacement(P0, P1), max_displacement(H0, H1))
    if displacement <= 1.0e-6:
        raise RuntimeError(f"ethnicity {race} has no character displacement ({displacement})")
    captures.append((P1, H1))
    proxynames.append(f"{race}-{SEX}-young")

set_ethnicity("caucasian")

# Append the face helper geometry after all proxy refits. Each morph then has
# one index across the body, eyes, teeth, and tongue in the exported mesh.
proxy_vertex_count = len(proxy.data.vertices)
helper_mesh = bpy.data.meshes.new("FaceHelpers")
helper_vertices = [
    tuple(H0[offset:offset + 3]) for offset in range(0, len(H0), 3)
]
helper_mesh.from_pydata(helper_vertices, [], face_helper_faces)
helper_mesh.update()
helper = bpy.data.objects.new("FaceHelpers", helper_mesh)
bpy.context.collection.objects.link(helper)

helper_groups = {}
for source_group in b.vertex_groups:
    helper_groups[source_group.index] = helper.vertex_groups.new(name=source_group.name)
for local, source in enumerate(face_helper_source_vertices):
    for membership in b.data.vertices[source].groups:
        helper_groups[membership.group].add([local], membership.weight, 'REPLACE')

skin_color = (0.58, 0.36, 0.22, 1.0)
proxy_colors = proxy.data.color_attributes.new(name="Color", type='FLOAT_COLOR', domain='CORNER')
for datum in proxy_colors.data:
    datum.color_srgb = skin_color
helper_colors = helper_mesh.color_attributes.new(name="Color", type='FLOAT_COLOR', domain='CORNER')
for loop in helper_mesh.loops:
    source = face_helper_source_vertices[loop.vertex_index]
    memberships = {b.vertex_groups[item.group].name for item in b.data.vertices[source].groups}
    if "helper-tongue" in memberships:
        color = (0.65, 0.12, 0.16, 1.0)
    elif "helper-lower-teeth" in memberships or "helper-upper-teeth" in memberships:
        color = (0.95, 0.92, 0.80, 1.0)
    else:
        color = (0.95, 0.95, 0.95, 1.0)
    helper_colors.data[loop.index].color_srgb = color

for item in bpy.context.scene.objects:
    item.select_set(False)
proxy.select_set(True)
helper.select_set(True)
bpy.context.view_layer.objects.active = proxy
bpy.ops.object.join()
if len(proxy.data.vertices) != proxy_vertex_count + len(face_helper_source_vertices):
    raise RuntimeError("face helper join changed the vertex count")
P0 = P0 + H0
captures = [proxy_capture + helper_capture for proxy_capture, helper_capture in captures]
print(f"> appended {len(face_helper_source_vertices)} face-helper vertices")

# The proxy source uses flat polygons. Smooth polygons and edges prevent the
# glTF exporter from making one normal and four vertices for each quad.
for polygon in proxy.data.polygons:
    polygon.use_smooth = True
for edge in proxy.data.edges:
    edge.use_edge_sharp = False
proxy.data.update()

basis = proxy.shape_key_add(name='Basis', from_mix=False)
basis.data.foreach_set("co", P0)
for name, coords in zip(proxynames, captures):
    key = proxy.shape_key_add(name=name, from_mix=False)
    key.data.foreach_set("co", coords)
    key.value = 0.0
proxy.active_shape_key_index = 0
bpy.context.view_layer.update()

print("> PROXY KEYS before export:", [k.name for k in proxy.data.shape_keys.key_blocks])
print(f"> transferred {len(proxynames)} morph targets onto the proxy")

# --- export proxy, hair, and rig ------------------------------------------
for o in bpy.context.scene.objects: o.select_set(False)
proxy.select_set(True)
for _, _, hair, _, _ in hair_assets:
    hair.select_set(True)
rig.select_set(True)
bpy.context.view_layer.objects.active = proxy
os.makedirs(os.path.dirname(OUT), exist_ok=True)
# sidecar: proxy morph names (glTF targets have no names)
names = [k.name for k in proxy.data.shape_keys.key_blocks][1:] if proxy.data.shape_keys else []
with open(OUT.replace(".glb", ".morphs.json"), "w") as f:
    json.dump(names, f)
with open(OUT.replace(".glb", ".controls.json"), "w") as f:
    json.dump(control_specs, f)
hair_fit = {
    "version": 3,
    "driverVertexCount": len(driver_sources),
    "driverNeutral": list(D0),
    "targets": driver_targets,
    "proxy": proxy_support_record,
    "styles": hair_style_records,
}
with open(OUT.replace(".glb", ".hair-fit.json"), "w") as f:
    json.dump(hair_fit, f, separators=(",", ":"))
bpy.ops.export_scene.gltf(filepath=OUT, export_format='GLB', use_selection=True,
    export_skins=True, export_morph=True, export_morph_normal=False,
    export_morph_tangent=False, export_apply=False, export_yup=True, export_materials='NONE')
print(
    f"> exported {OUT} with {len(names)} proxy morph targets, "
    f"{len(driver_sources)} hair drivers, and {len(driver_targets)} structural targets"
)
