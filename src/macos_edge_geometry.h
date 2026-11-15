#ifndef FIEVEL_MACOS_EDGE_GEOMETRY_H
#define FIEVEL_MACOS_EDGE_GEOMETRY_H

#include <CoreGraphics/CoreGraphics.h>
#include <float.h>
#include <math.h>
#include <stdbool.h>
#include <stddef.h>

#define FIEVEL_SEGMENT_EPSILON (64 * DBL_EPSILON)

// Core Graphics display bounds are half-open. Preserve positions already inside a
// display, but use the last whole point for an exposed right or bottom edge.
static CGPoint fievel_nearest_display_point(CGPoint point, const CGRect *displays, size_t count) {
    CGPoint nearest = point;
    double best = INFINITY;
    for (size_t i = 0; i < count; i++) {
        CGRect r = displays[i];
        if (CGRectIsEmpty(r)) continue;
        double min_x = CGRectGetMinX(r), max_x = CGRectGetMaxX(r);
        double min_y = CGRectGetMinY(r), max_y = CGRectGetMaxY(r);
        CGPoint p = CGPointMake(
            point.x >= max_x ? fmax(min_x, max_x - 1.0) : fmax(point.x, min_x),
            point.y >= max_y ? fmax(min_y, max_y - 1.0) : fmax(point.y, min_y)
        );
        double distance = hypot(point.x - p.x, point.y - p.y);
        if (distance < best) { best = distance; nearest = p; }
    }
    return nearest;
}

static bool fievel_segment_display_interval(CGPoint start, CGPoint delta, CGRect r, double *enter, double *exit) {
    if (CGRectIsEmpty(r)) return false;
    double low = 0.0, high = 1.0;
    double origin[] = {start.x, start.y};
    double motion[] = {delta.x, delta.y};
    double minimum[] = {CGRectGetMinX(r), CGRectGetMinY(r)};
    double maximum[] = {CGRectGetMaxX(r), CGRectGetMaxY(r)};
    for (int axis = 0; axis < 2; axis++) {
        if (motion[axis] == 0.0) {
            if (origin[axis] < minimum[axis] || origin[axis] >= maximum[axis]) return false;
        } else {
            double a = (minimum[axis] - origin[axis]) / motion[axis];
            double b = (maximum[axis] - origin[axis]) / motion[axis];
            low = fmax(low, fmin(a, b));
            high = fmin(high, fmax(a, b));
            if (low > high + FIEVEL_SEGMENT_EPSILON) return false;
        }
    }
    *exit = fmax(high, 0.0);
    *enter = fmin(fmin(low, 1.0), *exit);
    return true;
}

// Find the portion of the movement segment covered by connected display bounds.
// This allows crossing shared edges without jumping across gaps between displays.
static CGPoint fievel_clip_display_segment(CGPoint start, CGPoint delta, const CGRect *displays, size_t count, double *fraction) {
    double reachable = 0.0;
    bool changed;
    do {
        changed = false;
        for (size_t i = 0; i < count; i++) {
            double enter, exit;
            if (fievel_segment_display_interval(start, delta, displays[i], &enter, &exit) &&
                enter <= reachable + FIEVEL_SEGMENT_EPSILON && exit > reachable) {
                reachable = exit;
                changed = true;
            }
        }
    } while (changed && reachable < 1.0);
    *fraction = reachable;
    CGPoint destination = CGPointMake(start.x + delta.x * reachable, start.y + delta.y * reachable);
    CGPoint nearest = destination;
    double best = INFINITY;
    // Only project onto displays reached by the segment; a nearby disconnected
    // display must not pull the pointer across a narrow gap.
    for (size_t i = 0; i < count; i++) {
        double enter, exit;
        if (fievel_segment_display_interval(start, delta, displays[i], &enter, &exit) &&
            enter <= reachable + FIEVEL_SEGMENT_EPSILON && exit >= reachable - FIEVEL_SEGMENT_EPSILON) {
            // Merely touching an excluded corner does not enter a display.
            if (enter == exit && !CGRectContainsPoint(displays[i], destination)) continue;
            CGPoint candidate = fievel_nearest_display_point(destination, &displays[i], 1);
            CGRect r = displays[i];
            if ((delta.x > 0 && candidate.x < start.x) ||
                (delta.x < 0 && candidate.x > start.x)) {
                if (start.x < CGRectGetMinX(r) || start.x >= CGRectGetMaxX(r)) continue;
                candidate.x = start.x;
            }
            if ((delta.y > 0 && candidate.y < start.y) ||
                (delta.y < 0 && candidate.y > start.y)) {
                if (start.y < CGRectGetMinY(r) || start.y >= CGRectGetMaxY(r)) continue;
                candidate.y = start.y;
            }
            double distance = hypot(destination.x - candidate.x, destination.y - candidate.y);
            if (distance < best) { best = distance; nearest = candidate; }
        }
    }
    return nearest;
}

static CGPoint fievel_display_constrained_destination(CGPoint start, CGPoint delta, const CGRect *displays, size_t count) {
    if (count == 0) return CGPointMake(start.x + delta.x, start.y + delta.y);
    start = fievel_nearest_display_point(start, displays, count);
    double fraction;
    CGPoint point = fievel_clip_display_segment(start, delta, displays, count, &fraction);
    // Sliding along one edge may expose another display to the blocked axis.
    if (fraction < 1.0 && delta.x != 0.0 && delta.y != 0.0) {
        CGPoint target = CGPointMake(start.x + delta.x, start.y + delta.y);
        for (size_t i = 0; i < 2 * count + 1; i++) {
            CGPoint previous = point;
            double ignored;
            if (point.x != target.x) {
                point = fievel_clip_display_segment(point, CGPointMake(target.x - point.x, 0), displays, count, &ignored);
            }
            if (point.y != target.y) {
                point = fievel_clip_display_segment(point, CGPointMake(0, target.y - point.y), displays, count, &ignored);
            }
            if (point.x == previous.x && point.y == previous.y) break;
        }
    }
    return point;
}
#endif
