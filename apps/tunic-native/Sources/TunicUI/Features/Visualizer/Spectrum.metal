#include <metal_stdlib>
using namespace metal;

vertex float4 fieldVertex(uint id [[vertex_id]]) {
    const float2 corners[] = {float2(-1, -1), float2(3, -1), float2(-1, 3)};
    return float4(corners[id], 0, 1);
}

struct FieldParameters {
    float4 viewport; // logical width, height, backing scale, bin count
    float4 halftone; // maximum cell size, center spacing, level influence, circle/square
    float4 response; // size response, opacity response, solid color flag, padding
    float4 solidColor;
    float4 gradientStart;
    float4 gradientSecond;
    float4 gradientThird;
    float4 gradientEnd;
};

float2 logicalPoint(float2 clip, float2 size) {
    return float2(clip.x + 1, 1 - clip.y) * 0.5 * size;
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
    float x = clamp(p.x / parameters.viewport.x, 0.0f, 1.0f) * 3;
    float3 color = x < 1
        ? mix(parameters.gradientStart.rgb, parameters.gradientSecond.rgb, x)
        : x < 2 ? mix(parameters.gradientSecond.rgb, parameters.gradientThird.rgb, x - 1)
                : mix(parameters.gradientThird.rgb, parameters.gradientEnd.rgb, x - 2);
    if (parameters.response.z > 0.5) color = parameters.solidColor.rgb;
    return float4(color * coverage, coverage);
}

fragment float4 fieldFragment(float4 position [[position]],
                              const device float2 *points [[buffer(0)]],
                              constant FieldParameters &parameters [[buffer(1)]]) {
    float scale = parameters.viewport.z;
    float2 p = position.xy / scale;
    return shadeHalftone(p, points, parameters);
}
