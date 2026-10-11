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
    float4 halftone; // maximum cell size, center spacing, level influence, circle/square
    float4 variation; // padding, bass-driven channel shift in points, solid color flag, inverted flag
    float4 response; // size response, opacity response, padding
    float4 solidColor;
    float4 gradientStart;
    float4 gradientSecond;
    float4 gradientThird;
    float4 gradientEnd;
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

float squareSegmentDistance(float2 delta, float2 edge) {
    // Minimize L-infinity distance along the segment: endpoints or where |x| = |y|.
    float nearest = min(max(abs(delta.x), abs(delta.y)),
                        max(abs(delta.x - edge.x), abs(delta.y - edge.y)));
    for (int sign = -1; sign <= 1; sign += 2) {
        float denominator = edge.x - sign * edge.y;
        float t = denominator == 0 ? 0 : clamp((delta.x - sign * delta.y) / denominator, 0.0f, 1.0f);
        float2 offset = abs(delta - t * edge);
        nearest = min(nearest, max(offset.x, offset.y));
    }
    return nearest;
}

// Cells only need clearance up to their half-size + AA, not the full SDF.
// With monotonic, uniformly spaced x bins, segments outside this x interval
// cannot be nearer. Include both segments straddling the interval endpoints.
float spectrumClearance(float2 p, float limit, const device float2 *points, uint count, float2 size,
                        bool square = false) {
    if (p.y <= size.y * (1 - spectrumHeight(p.x, points, count, size.x))) return 0;
    float firstX = logicalPoint(points[0], size).x;
    float step = (points[1].x - points[0].x) * 0.5 * size.x;
    uint first = uint(clamp(floor((p.x - limit - firstX) / step), 0.0f, float(count - 2)));
    uint last = uint(clamp(floor((p.x + limit - firstX) / step), 0.0f, float(count - 2)));
    float nearestSquared = limit * limit;
    float2 a = logicalPoint(points[first], size);
    for (uint i = first; i <= last; ++i) {
        float2 b = logicalPoint(points[i + 1], size);
        float2 edge = b - a;
        float t = clamp(dot(p - a, edge) / dot(edge, edge), 0.0f, 1.0f);
        float2 delta = p - (a + t * edge);
        float distanceSquared = dot(delta, delta);
        if (square) {
            float distance = squareSegmentDistance(p - a, edge);
            distanceSquared = distance * distance;
        }
        nearestSquared = min(nearestSquared, distanceSquared);
        a = b;
    }
    return sqrt(nearestSquared);
}

float4 shadeHalftone(float2 p, const device float2 *points,
                     constant FieldParameters &parameters) {
    float inside = p.y - parameters.viewport.y * (1 - spectrumHeight(p.x, points, uint(parameters.viewport.w), parameters.viewport.x));
    if (inside <= 0) return float4(0);
    float spacing = parameters.halftone.y;
    bool square = parameters.halftone.w > 0.5;
    float rowStep = spacing;
    float aa = 0.75 / parameters.viewport.z;
    float reach = 0.5 * parameters.halftone.x + aa;
    // Anchor to the bottom so the lattice stays fixed as the spectrum moves.
    float2 q = float2(p.x, parameters.viewport.y - p.y);
    float coverage = 0;
    // Visit every center whose maximum radius could reach this pixel. Overlapping
    // dots can extend beyond immediate neighbors; small dots keep a small search.
    int firstRow = max(0, int(ceil((q.y - reach - spacing * 0.5) / rowStep)));
    int lastRow = int(floor((q.y + reach - spacing * 0.5) / rowStep));
    for (int r = firstRow; r <= lastRow; ++r) {
        int firstColumn = int(ceil((q.x - reach - spacing * 0.5) / spacing));
        int lastColumn = int(floor((q.x + reach - spacing * 0.5) / spacing));
        for (int c = firstColumn; c <= lastColumn; ++c) {
            float2 center = float2((c + 0.5) * spacing, spacing * 0.5 + r * rowStep);
            float height = spectrumHeight(center.x, points, uint(parameters.viewport.w), parameters.viewport.x);
            if (height <= 0) continue;
            // Evaluate at the dot center to keep circles round. Normalize within this
            // frequency's filled area, not the whole viewport: 0 at bottom, 1 at curve.
            float position = clamp(center.y / (height * parameters.viewport.y), 0.0f, 1.0f);
            float response = mix(1.0f, height, parameters.halftone.z) * position;
            float radius = 0.5 * parameters.halftone.x * mix(1.0f, response, parameters.response.x);
            float opacity = mix(1.0f, response, parameters.response.y);
            float2 offset = abs(q - center);
            float fromCenter = square ? max(offset.x, offset.y) : length(offset);
            if (radius <= 0 || fromCenter >= radius + aa) continue;
            // Fit the entire cell and its AA fringe. Euclidean clearance fits
            // circles; L-infinity clearance fits square corners, even in valleys.
            float2 screenCenter = float2(center.x, parameters.viewport.y - center.y);
            float2 edge = min(screenCenter, parameters.viewport.xy - screenCenter);
            float limit = min(radius + aa, min(edge.x, edge.y));
            if (limit <= aa) continue;
            float clearance = spectrumClearance(screenCenter, limit, points,
                                                uint(parameters.viewport.w), parameters.viewport.xy, square);
            radius = min(radius, clearance - aa);
            if (radius <= 0) continue;
            float dot = 1 - smoothstep(radius - aa, radius + aa, fromCenter);
            coverage = max(coverage, dot * opacity);
        }
    }
    float alpha = parameters.variation.w > 0.5
        ? (1 - coverage) * smoothstep(0.0f, 2 * aa, inside) : coverage;
    float x = clamp(p.x / parameters.viewport.x, 0.0f, 1.0f) * 3;
    float3 color = x < 1
        ? mix(parameters.gradientStart.rgb, parameters.gradientSecond.rgb, x)
        : x < 2 ? mix(parameters.gradientSecond.rgb, parameters.gradientThird.rgb, x - 1)
                : mix(parameters.gradientThird.rgb, parameters.gradientEnd.rgb, x - 2);
    if (parameters.variation.z > 0.5) color = parameters.solidColor.rgb;
    return float4(color * alpha, alpha);
}

float4 sampleHalftone(float2 p, const device float2 *points,
                      constant FieldParameters &parameters) {
    if (any(p < 0) || any(p >= parameters.viewport.xy)) return float4(0);
    return shadeHalftone(p, points, parameters);
}

// Composition order: channel offsets → halftone coverage/color.
float4 chromaticHalftone(float2 p, const device float2 *points,
                         constant FieldParameters &parameters) {
    float4 base = sampleHalftone(p, points, parameters);
    float shift = parameters.variation.y;
    if (shift <= 0) return base;
    // A single bass envelope displaces channels across the whole image, including
    // its spectrum boundary. It does not change the dots only in the bass region.
    float4 red = sampleHalftone(p - float2(shift, 0), points, parameters);
    float4 blue = sampleHalftone(p + float2(shift, 0), points, parameters);
    float3 rgb = float3(red.r, base.g, blue.b);
    // Coverage is independent of color brightness, including dark/black solid fills.
    return float4(rgb, max(red.a, max(base.a, blue.a)));
}

fragment float4 fieldFragment(float4 position [[position]],
                              const device float2 *points [[buffer(0)]],
                              constant FieldParameters &parameters [[buffer(1)]]) {
    float scale = parameters.viewport.z;
    float2 p = position.xy / scale;
    uint mode = uint(parameters.mapping.w);
    if (mode == 4) return chromaticHalftone(p, points, parameters);
    float d = spectrumDistance(p, points, uint(parameters.viewport.w), parameters.viewport.xy);
    float4 color = shadeDistance(d, parameters.mapping, scale);
    if (mode == 1 || mode == 2) {
        // Fade extended effects into the host instead of exposing a rectangular crop.
        float2 edge = min(p, parameters.viewport.xy - p);
        color *= smoothstep(0.0f, 10.0f, min(edge.x, edge.y));
    }
    return color;
}

float3 glowLinear(float3 color) {
    return select(color / 12.92f, pow((color + 0.055f) / 1.055f, float3(2.4f)), color > 0.04045f);
}

float3 glowSRGB(float3 color) {
    return select(color * 12.92f, 1.055f * pow(max(color, 0.0f), float3(1.0f / 2.4f)) - 0.055f, color > 0.0031308f);
}

float4 glowDecode(float4 color) {
    return float4(glowLinear(color.rgb / max(color.a, 0.000001f)) * color.a, color.a);
}

// Post-process the already shaded/chromatically split image in linear light.
fragment float4 bloomBlur(float4 position [[position]],
                          texture2d<float> source [[texture(0)]],
                          constant float4 &parameters [[buffer(0)]],
                          constant uint &decode [[buffer(1)]]) {
    constexpr sampler linearSampler(coord::normalized, address::clamp_to_zero, filter::linear);
    float2 uv = position.xy / parameters.xy;
    float4 sum = float4(0);
    float total = 0;
    for (int tap = -6; tap <= 6; ++tap) {
        float weight = exp(-float(tap * tap) / 8.0f);
        float4 sample = source.sample(linearSampler, uv + float(tap) * parameters.zw);
        sum += (decode ? glowDecode(sample) : sample) * weight;
        total += weight;
    }
    return sum / total;
}

fragment float4 bloomComposite(float4 position [[position]],
                               texture2d<float> scene [[texture(0)]],
                               array<texture2d<float>, 5> levels [[texture(1)]],
                               constant float4 &parameters [[buffer(0)]]) {
    constexpr sampler linearSampler(coord::normalized, address::clamp_to_zero, filter::linear);
    float2 uv = position.xy / parameters.xy;
    float4 base = scene.read(uint2(position.xy));
    // Equal-energy normalized Gaussians at octave-spaced radii approximate 1/r²:
    // integral G_sigma(r) d(log sigma) is proportional to 1/r². Finite scales
    // soften the singular core and truncate the tail; this is not the plugin's kernel.
    float4 glow = float4(0);
    for (uint level = 0; level < 5; ++level) glow += levels[level].sample(linearSampler, uv) / 5.0f;
    glow *= parameters.z;
    // Fade only the halo at the viewport edge to avoid a rectangular glow crop.
    float2 edge = min(position.xy, parameters.xy - position.xy) / parameters.w;
    glow *= smoothstep(0.0f, 8.0f, min(edge.x, edge.y));
    // Smooth exposure rolloff, then composite behind the crisp source in linear light.
    glow = 1 - exp(-glow);
    float4 result = glowDecode(base) + glow * (1 - base.a);
    return float4(glowSRGB(result.rgb / max(result.a, 0.000001f)) * result.a, result.a);
}
