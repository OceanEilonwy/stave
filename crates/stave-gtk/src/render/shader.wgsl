// The Stave viewport: wood and brushed metal under a warm key light, a
// cool fill and a hemisphere ambient; a soft ground shadow; thin dark
// outlines. Colours are linear; the target is sRGB.

struct Globals {
    view_projection: mat4x4<f32>,
    light_view_projection: mat4x4<f32>,
    eye: vec4<f32>,
    // xyz: unit vector toward the key light.
    key_direction: vec4<f32>,
    // xyz: unit vector toward the fill light.
    fill_direction: vec4<f32>,
    // x, z: centre of the floor; y: its height; w: half its size.
    floor: vec4<f32>,
    // xyz: a section's unit normal, toward the side cut away (zero: none);
    // w: its offset.
    section: vec4<f32>,
    // rgb: the ground shadow's colour; a: its strength.
    shadow: vec4<f32>,
};

// Is a point cut away by the section?
fn cut_away(world: vec3<f32>) -> bool {
    let normal = globals.section.xyz;
    return dot(normal, normal) > 0.5 && dot(normal, world) > globals.section.w;
}

struct Node {
    model: mat4x4<f32>,
    // rgb: base colour; a: how strongly the grain shows.
    colour: vec4<f32>,
    // xy: pith in grain coordinates; z: a seed; w: 0 wood, 1 metal.
    params: vec4<f32>,
    // rgb: highlight colour; a: how much of it.
    tint: vec4<f32>,
    // x: opacity; y: fade toward a pale grey; z: how much darker faces
    // cut by a joint are; w: outline strength.
    style: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> nodes: array<Node>;
@group(1) @binding(0) var shadow_map: texture_depth_2d;
@group(1) @binding(1) var shadow_sampler: sampler_comparison;

const SKY: vec3<f32> = vec3<f32>(1.0, 0.96, 0.90);
const GROUND: vec3<f32> = vec3<f32>(0.45, 0.37, 0.27);
const KEY_COLOUR: vec3<f32> = vec3<f32>(1.0, 0.92, 0.80);
const FILL_COLOUR: vec3<f32> = vec3<f32>(0.78, 0.84, 0.97);
const AMBIENT: f32 = 0.50;
const KEY: f32 = 0.78;
const FILL: f32 = 0.26;
// How much darker the faces a cut made are drawn.
const CUT_TINT: f32 = 0.38;

// ---- noise ----

fn hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(0.1031, 0.1030));
    let r = q + dot(q, q.yx + 33.33);
    return fract((r.x + r.y) * r.x);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash(i);
    let b = hash(i + vec2<f32>(1.0, 0.0));
    let c = hash(i + vec2<f32>(0.0, 1.0));
    let d = hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// ---- meshes ----

struct MeshIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) grain: vec3<f32>,
    @location(3) cut: f32,
};

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) grain: vec3<f32>,
    @location(3) @interpolate(flat) node: u32,
    @location(4) cut: f32,
};

@vertex
fn mesh_vertex(in: MeshIn, @builtin(instance_index) node: u32) -> MeshOut {
    let model = nodes[node].model;
    let world = model * vec4<f32>(in.position, 1.0);
    var out: MeshOut;
    out.clip = globals.view_projection * world;
    out.world = world.xyz;
    out.normal = (model * vec4<f32>(in.normal, 0.0)).xyz;
    out.grain = in.grain;
    out.node = node;
    out.cut = in.cut;
    return out;
}

// How much of the key light reaches a point: 1 lit, 0 in shadow (soft).
fn key_light(world: vec3<f32>, normal: vec3<f32>) -> f32 {
    let light = globals.light_view_projection * vec4<f32>(world, 1.0);
    let ndc = light.xyz / light.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let facing = max(dot(normal, globals.key_direction.xyz), 0.0);
    let bias = 0.0015 + 0.003 * (1.0 - facing);
    let texel = 1.0 / 2048.0;
    var lit = 0.0;
    for (var x = -2; x <= 2; x += 1) {
        for (var y = -2; y <= 2; y += 1) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel * 1.5;
            lit += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + offset, ndc.z - bias);
        }
    }
    lit = lit / 25.0;
    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0)) && ndc.z <= 1.0;
    return select(1.0, lit, inside);
}

// The distance from the tree's centre, in millimetres: growth rings are
// circles of it. Worked out outside any branch so its screen derivative
// is defined.
fn ring_radius(grain: vec3<f32>, params: vec4<f32>) -> f32 {
    let seed = params.z;
    let across = vec2<f32>(grain.y - params.x, grain.z - params.y);
    // The rings wander gently along the length.
    let wander = noise(vec2<f32>(grain.x * 0.0018, seed)) * 3.5
        + noise(vec2<f32>(grain.x * 0.007, seed + 5.0)) * 0.8;
    return length(across) + wander;
}

fn wood(base: vec3<f32>, grain: vec3<f32>, params: vec4<f32>, figure: f32, radius: f32, blur: f32) -> vec3<f32> {
    let seed = params.z;
    let spacing = 5.5;
    let ring = fract(radius / spacing + noise(vec2<f32>(radius * 0.05, seed + 9.0)) * 0.9);
    let late = smoothstep(0.50, 0.85, ring) * (1.0 - smoothstep(0.88, 1.0, ring));
    // Rings finer than a couple of pixels fade to their average, so
    // distant faces don't shimmer.
    let resolved = 1.0 - smoothstep(0.15, 0.45, blur / spacing);
    let rings = mix(0.30, late, resolved);
    // Fine streaks along the grain: long along, narrow across.
    let streak = noise(vec2<f32>(grain.x * 0.015, radius * 1.3 + seed));
    let broad = noise(vec2<f32>(grain.x * 0.0015, radius * 0.04 + seed * 3.0));
    let shade = 1.0 - figure * (0.15 * rings + 0.05 * streak * resolved + 0.07 * broad);
    return base * shade;
}

fn metal(base: vec3<f32>, grain: vec3<f32>) -> vec3<f32> {
    // Brushed along the length.
    let brushed = noise(vec2<f32>(grain.x * 0.08, grain.y * 6.0 + grain.z * 6.0));
    return base * (0.90 + 0.14 * brushed);
}

@fragment
fn mesh_fragment(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if (cut_away(in.world)) {
        discard;
    }
    let node = nodes[in.node];
    var normal = normalize(in.normal);
    // A section's cut face is lit evenly: the surface drawn there lies
    // below the cut, so its own shadow would streak it.
    var cut_face = false;
    if (!front) {
        normal = -normal;
        // Seen through a section's cut: the inside of the wood, drawn as
        // the flat cut face, facing the way the section looks.
        if (dot(globals.section.xyz, globals.section.xyz) > 0.5) {
            normal = globals.section.xyz;
            cut_face = true;
        }
    }
    let is_metal = node.params.w > 0.5;
    let radius = ring_radius(in.grain, node.params);
    let blur = fwidth(radius);
    var albedo = wood(node.colour.rgb, in.grain, node.params, node.colour.a, radius, blur);
    if (is_metal) {
        albedo = metal(node.colour.rgb, in.grain);
    } else {
        // Faces a cut made are tinted darker, so the joinery reads.
        albedo = albedo * (1.0 - clamp(CUT_TINT * node.style.z * in.cut, 0.0, 0.8));
    }
    let to_eye = normalize(globals.eye.xyz - in.world);
    let key_direction = globals.key_direction.xyz;
    var shadow = key_light(in.world, normal);
    if (cut_face) {
        shadow = 1.0;
    }
    let hemisphere = mix(GROUND, SKY, normal.y * 0.5 + 0.5) * AMBIENT;
    let key = max(dot(normal, key_direction), 0.0) * KEY * shadow * KEY_COLOUR;
    let fill = max(dot(normal, globals.fill_direction.xyz), 0.0) * FILL * FILL_COLOUR;
    var colour = albedo * (hemisphere + key + fill);
    let halfway = normalize(key_direction + to_eye);
    if (is_metal) {
        let specular = pow(max(dot(normal, halfway), 0.0), 36.0) * 0.55 * shadow;
        let sheen = pow(1.0 - max(dot(normal, to_eye), 0.0), 3.0) * 0.12;
        colour += vec3<f32>(specular + sheen) * KEY_COLOUR;
    } else {
        let specular = pow(max(dot(normal, halfway), 0.0), 24.0) * 0.05 * shadow;
        colour += vec3<f32>(specular) * KEY_COLOUR;
    }
    // Selection: the accent colour, lit like everything else, with a
    // brighter rim.
    let rim = pow(1.0 - max(dot(normal, to_eye), 0.0), 2.0);
    let accent = node.tint.rgb * (hemisphere + key + fill) * 1.25 + vec3<f32>(0.18 * rim);
    colour = mix(colour, accent, node.tint.a);
    // A ghost fades toward a pale warm grey.
    colour = mix(colour, vec3<f32>(0.80, 0.78, 0.75), node.style.y);
    // Seen through (X-ray, not a ghost), the faces a cut made stay
    // nearly solid, so a socket or a mortise reads inside its part.
    var alpha = node.style.x;
    if (node.style.x < 0.99 && node.style.y < 0.5) {
        alpha = mix(alpha, 0.88, in.cut);
    }
    return vec4<f32>(colour, alpha);
}

// ---- shadow map ----

@vertex
fn shadow_vertex(in: MeshIn, @builtin(instance_index) node: u32) -> @builtin(position) vec4<f32> {
    return globals.light_view_projection * nodes[node].model * vec4<f32>(in.position, 1.0);
}

// ---- outlines ----

struct EdgeOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) node: u32,
    @location(1) world: vec3<f32>,
};

@vertex
fn edge_vertex(@location(0) position: vec3<f32>, @builtin(instance_index) node: u32) -> EdgeOut {
    let world = nodes[node].model * vec4<f32>(position, 1.0);
    // Faces are drawn with a depth bias, so a line wins against its own
    // faces without poking through thin parts in front of it.
    var out: EdgeOut;
    out.clip = globals.view_projection * world;
    out.node = node;
    out.world = world.xyz;
    return out;
}

@fragment
fn edge_fragment(in: EdgeOut) -> @location(0) vec4<f32> {
    if (cut_away(in.world)) {
        discard;
    }
    let node = nodes[in.node];
    let ink = vec4<f32>(0.022, 0.012, 0.005, 0.42 * node.style.w);
    let selected = vec4<f32>(node.tint.rgb * 0.6, 1.0);
    return mix(ink, selected, step(0.01, node.tint.a));
}

// ---- floor: a ground plane that only shows the shadow ----

struct FloorOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn floor_vertex(@builtin(vertex_index) index: u32) -> FloorOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[index];
    let floor = globals.floor;
    let world = vec3<f32>(floor.x + corner.x * floor.w, floor.y, floor.z + corner.y * floor.w);
    var out: FloorOut;
    out.clip = globals.view_projection * vec4<f32>(world, 1.0);
    out.world = world;
    return out;
}

@fragment
fn floor_fragment(in: FloorOut) -> @location(0) vec4<f32> {
    let shadow = 1.0 - key_light(in.world, vec3<f32>(0.0, 1.0, 0.0));
    // Fade out toward the edge of the floor.
    let away = length(in.world.xz - globals.floor.xz) / globals.floor.w;
    let fade = 1.0 - smoothstep(0.5, 1.0, away);
    return vec4<f32>(globals.shadow.rgb, shadow * globals.shadow.a * fade);
}

// ---- a problem's shared volumes: red, on top of everything ----

@vertex
fn volume_vertex(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return globals.view_projection * vec4<f32>(position, 1.0);
}

@fragment
fn volume_fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(0.95, 0.0, 0.0, 0.62);
}

@fragment
fn volume_edge_fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(0.45, 0.0, 0.0, 0.95);
}

// ---- the overlay: paths, outlines and labels, on screen ----

struct OverlayOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec4<f32>,
};

@vertex
fn overlay_vertex(@location(0) position: vec2<f32>, @location(1) colour: vec4<f32>) -> OverlayOut {
    var out: OverlayOut;
    out.clip = vec4<f32>(position, 0.0, 1.0);
    out.colour = colour;
    return out;
}

@fragment
fn overlay_fragment(in: OverlayOut) -> @location(0) vec4<f32> {
    return in.colour;
}
