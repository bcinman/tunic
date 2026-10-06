#include <metal_stdlib>
using namespace metal;

struct Point {
    float4 position [[position]];
    float size [[point_size]];
};

vertex Point spectrumVertex(uint id [[vertex_id]],
                            const device float2 *points [[buffer(0)]],
                            constant float &size [[buffer(1)]]) {
    return {float4(points[id], 0, 1), size};
}

fragment float4 spectrumFragment(float2 uv [[point_coord]]) {
    if (distance(uv, float2(0.5)) > 0.5) discard_fragment();
    return float4(1);
}

vertex float4 fieldVertex(uint id [[vertex_id]]) {
    const float2 corners[] = {float2(-1, -1), float2(3, -1), float2(-1, 3)};
    return float4(corners[id], 0, 1);
}

struct FieldParameters {
    float4 viewport; // logical width, height, backing scale, bin count
    float4 mapping;  // stroke width, spread, hue, shading mode
    float4 halftone; // maximum diameter, center spacing, amplitude response, grid/hex
    float4 variation; // signed vertical response, bass-driven channel shift in points, white color flag, unused
    float4 ripple; // vertical displacement in points, wave half-width in points, unused, unused
    float4 waves[4]; // front x in points, remaining energy, unused, unused
};

float2 logicalPoint(float2 clip, float2 size) {
    return float2(clip.x + 1, 1 - clip.y) * 0.5 * size;
}

// Layer 1: Euclidean distance to the piecewise-linear spectrum, in logical points.
// Positive above the curve, negative below. The curve is a graph, not a closed fill.
// This prototype scans all segments for correctness on steep slopes.
float spectrumDistance(float2 p, const device float2 *points, uint count, float2 size) {
    float nearest = INFINITY;
    float side = 1;
    float2 a = logicalPoint(points[0], size);
    for (uint i = 1; i < count; ++i) {
        float2 b = logicalPoint(points[i], size);
        float2 edge = b - a;
        float t = clamp(dot(p - a, edge) / dot(edge, edge), 0.0f, 1.0f);
        nearest = min(nearest, length(p - (a + t * edge)));
        if (p.x >= a.x && p.x <= b.x) {
            float curveY = mix(a.y, b.y, (p.x - a.x) / (b.x - a.x));
            side = p.y <= curveY ? 1.0f : -1.0f;
        }
        a = b;
    }
    return nearest * side;
}

float3 hueColor(float hue) {
    return clamp(abs(fract(hue + float3(0, 2.0 / 3.0, 1.0 / 3.0)) * 6 - 3) - 1,
                 0.0f, 1.0f);
}

// Layer 2: map the same signed distance to different appearances.
// Return premultiplied alpha so the SwiftUI material remains visible underneath.
float4 shadeDistance(float d, float4 mapping, float scale) {
    float width = mapping.x;
    float spread = mapping.y;
    float hue = mapping.z;
    uint mode = uint(mapping.w);
    float aa = 0.75 / scale;
    float line = 1 - smoothstep(width * 0.5 - aa, width * 0.5 + aa, abs(d));
    float3 color = hueColor(hue);
    float alpha = line;
    switch (mode) {
        case 1: // Glow
            alpha = max(line, 0.65 * exp(-abs(d) / spread));
            color = mix(color, float3(1), line * 0.65);
            break;
        case 2: { // Contour bands on both sides of the spectrum
            float ring = abs(fract(d / spread + 0.5) - 0.5) * spread;
            alpha = (1 - smoothstep(width * 0.5 - aa, width * 0.5 + aa, ring))
                * exp(-abs(d) / (spread * 3));
            color = hueColor(hue + d / (spread * 6));
            break;
        }
        case 3: // Debug sign and magnitude: cyan above, orange below, white at zero
            color = d >= 0 ? float3(0.1, 0.8, 1) : float3(1, 0.35, 0.1);
            color = mix(float3(1), color, clamp(abs(d) / spread, 0.0f, 1.0f));
            alpha = 0.75;
            break;
    }
    return float4(color * alpha, alpha);
}

// The bins are uniformly spaced along logarithmic x, including when the view crops them.
float spectrumHeight(float x, const device float2 *points, uint count, float width) {
    float clipX = 2 * x / width - 1;
    float bin = clamp((clipX - points[0].x) / (points[1].x - points[0].x), 0.0f, float(count - 1));
    uint index = min(uint(bin), count - 2);
    return clamp(0.5 * (mix(points[index].y, points[index + 1].y, bin - index) + 1), 0.0f, 1.0f);
}

float4 shadeHalftone(float2 p, float d, const device float2 *points,
                     constant FieldParameters &parameters) {
    if (d >= 0) return float4(0);
    float spacing = parameters.halftone.y;
    bool hex = parameters.halftone.w > 0.5;
    float rowStep = spacing * (hex ? sqrt(3.0f) * 0.5 : 1.0f);
    float aa = 0.75 / parameters.viewport.z;
    // Anchor to the bottom so the lattice stays fixed as the spectrum moves.
    float2 q = float2(p.x, parameters.viewport.y - p.y);
    int row = int(round((q.y - spacing * 0.5) / rowStep));
    float coverage = 0;
    // Neighboring cells matter with hex rows and amplitude-dependent radii.
    for (int r = row - 1; r <= row + 1; ++r) {
        if (r < 0) continue;
        float offset = hex && (r % 2 != 0) ? spacing * 0.5 : 0;
        int column = int(round((q.x - spacing * 0.5 - offset) / spacing));
        for (int c = column - 1; c <= column + 1; ++c) {
            float2 center = float2((c + 0.5) * spacing + offset, spacing * 0.5 + r * rowStep);
            float height = spectrumHeight(center.x, points, uint(parameters.viewport.w), parameters.viewport.x);
            if (height <= 0) continue;
            // Evaluate at the dot center to keep circles round. Normalize within this
            // frequency's filled area, not the whole viewport: 0 at bottom, 1 at curve.
            float vertical = parameters.variation.x;
            float position = clamp(center.y / (height * parameters.viewport.y), 0.0f, 1.0f);
            float taper = mix(1.0f, vertical >= 0 ? position : 1 - position, abs(vertical));
            float radius = 0.5 * min(parameters.halftone.x, spacing)
                * mix(1.0f, height, parameters.halftone.z) * taper;
            float fromCenter = distance(q, center);
            if (radius <= 0 || fromCenter >= radius + aa) continue;
            // Fit the entire circle (including its antialias fringe) inside the
            // graph. Perpendicular distance also handles steep slopes and valleys;
            // vertical clearance alone would still slice the sides of a dot.
            float2 screenCenter = float2(center.x, parameters.viewport.y - center.y);
            float clearance = -spectrumDistance(screenCenter, points, uint(parameters.viewport.w), parameters.viewport.xy);
            float2 edge = min(screenCenter, parameters.viewport.xy - screenCenter);
            clearance = min(clearance, min(edge.x, edge.y));
            radius = min(radius, clearance - aa);
            if (radius <= 0) continue;
            float dot = 1 - smoothstep(radius - aa, radius + aa, fromCenter);
            coverage = max(coverage, dot);
        }
    }
    float alpha = coverage;
    float3 color = parameters.variation.z > 0.5 ? float3(1) : hueColor(parameters.mapping.z);
    return float4(color * alpha, alpha);
}

float4 sampleHalftone(float2 p, const device float2 *points,
                      constant FieldParameters &parameters) {
    // Warp the completed halftone's sampling coordinates: dots and silhouette
    // bend together, rather than resizing dots or clipping against an unwarped fill.
    float displacement = 0;
    if (parameters.ripple.x > 0) {
        for (uint wave = 0; wave < 4; ++wave) {
            float phase = (p.x - parameters.waves[wave].x) / parameters.ripple.y;
            float envelope = 1 - smoothstep(0.0f, 1.0f, abs(phase));
            displacement += parameters.waves[wave].y * sin(M_PI_F * phase) * envelope;
        }
    }
    p.y -= parameters.ripple.x * displacement;
    if (any(p < 0) || any(p >= parameters.viewport.xy)) return float4(0);
    float d = spectrumDistance(p, points, uint(parameters.viewport.w), parameters.viewport.xy);
    return shadeHalftone(p, d, points, parameters);
}

fragment float4 fieldFragment(float4 position [[position]],
                              const device float2 *points [[buffer(0)]],
                              constant FieldParameters &parameters [[buffer(1)]]) {
    float scale = parameters.viewport.z;
    float2 p = position.xy / scale;
    uint mode = uint(parameters.mapping.w);
    if (mode == 4) {
        float4 base = sampleHalftone(p, points, parameters);
        float shift = parameters.variation.y;
        if (shift <= 0) return base;
        // A single bass envelope displaces channels across the whole image, including
        // its spectrum boundary. It does not change the dots only in the bass region.
        float4 red = sampleHalftone(p - float2(shift, 0), points, parameters);
        float4 blue = sampleHalftone(p + float2(shift, 0), points, parameters);
        float3 rgb = float3(red.r, base.g, blue.b);
        // Preserve transparent gaps without dark fringes from an absent color channel.
        return float4(rgb, max(rgb.r, max(rgb.g, rgb.b)));
    }
    float d = spectrumDistance(p, points, uint(parameters.viewport.w), parameters.viewport.xy);
    float4 color = shadeDistance(d, parameters.mapping, scale);
    if (mode == 1 || mode == 2) {
        // Fade extended effects into the host instead of exposing a rectangular crop.
        float2 edge = min(p, parameters.viewport.xy - p);
        color *= smoothstep(0.0f, 10.0f, min(edge.x, edge.y));
    }
    return color;
}
