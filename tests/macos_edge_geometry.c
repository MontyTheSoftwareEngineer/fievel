#include <assert.h>
#include <math.h>
#include <stdio.h>
#include "../src/macos_edge_geometry.h"

static void expect_point(CGPoint actual, double x, double y) {
    assert(fabs(actual.x - x) < 1e-9);
    assert(fabs(actual.y - y) < 1e-9);
}

int main(void) {
    // Unequal displays, negative origins, exposed edges, and a shared edge.
    CGRect displays[] = {CGRectMake(0, 0, 200, 120), CGRectMake(-100, -20, 100, 100)};
    CGPoint p = fievel_display_constrained_destination(CGPointMake(50, 5), CGPointMake(0, -15), displays, 2);
    expect_point(p, 50, 0);
    p = fievel_display_constrained_destination(p, CGPointMake(0, -15), displays, 2);
    expect_point(p, 50, 0);
    p = fievel_display_constrained_destination(p, CGPointMake(0, 7), displays, 2);
    expect_point(p, 50, 7);
    p = fievel_display_constrained_destination(CGPointMake(5, 50), CGPointMake(-10, 0), displays, 2);
    expect_point(p, -5, 50);
    p = fievel_display_constrained_destination(CGPointMake(-5, 50), CGPointMake(10, 0), displays, 2);
    expect_point(p, 5, 50);
    p = fievel_display_constrained_destination(CGPointMake(5, 100), CGPointMake(-10, 0), displays, 2);
    expect_point(p, 0, 100);
    p = fievel_display_constrained_destination(CGPointMake(50, 0), CGPointMake(20, -20), displays, 2);
    expect_point(p, 70, 0);
    p = fievel_display_constrained_destination(CGPointMake(50, 119), CGPointMake(0, 20), displays, 2);
    expect_point(p, 50, 119);
    p = fievel_display_constrained_destination(CGPointMake(-50, -10), CGPointZero, displays, 2);
    expect_point(p, -50, -10);
    p = fievel_display_constrained_destination(CGPointMake(50, 119.5), CGPointZero, displays, 2);
    expect_point(p, 50, 119.5);
    CGRect gap[] = {CGRectMake(0, 0, 100, 100), CGRectMake(110, 0, 100, 100)};
    p = fievel_display_constrained_destination(CGPointMake(90, 50), CGPointMake(40, 0), gap, 2);
    expect_point(p, 99, 50);
    CGRect narrow_gap[] = {CGRectMake(0, 0, 100, 100), CGRectMake(100.5, 0, 100, 100)};
    p = fievel_display_constrained_destination(CGPointMake(90, 50), CGPointMake(40, 0), narrow_gap, 2);
    expect_point(p, 99, 50);
    CGRect stepped[] = {CGRectMake(0, 0, 100, 100), CGRectMake(0, 100, 50, 100)};
    p = fievel_display_constrained_destination(CGPointMake(50, 95), CGPointMake(0, 20), stepped, 2);
    expect_point(p, 50, 99);
    CGRect corners[] = {CGRectMake(0, 0, 100, 100), CGRectMake(100, 100, 100, 100)};
    p = fievel_display_constrained_destination(CGPointMake(105, 100), CGPointMake(-20, 0), corners, 2);
    expect_point(p, 100, 100);
    CGRect joined[] = {CGRectMake(0, 0, 100, 100), CGRectMake(100, 0, 100, 100)};
    p = fievel_display_constrained_destination(CGPointMake(99.3, 50), CGPointMake(0.7, 0), joined, 2);
    expect_point(p, 100, 50);
    CGRect fractional[] = {CGRectMake(-100.5, 0, 100, 100), CGRectMake(1000, 0, 100, 100)};
    p = fievel_display_constrained_destination(CGPointMake(-1, 50), CGPointMake(10, 0), fractional, 2);
    expect_point(p, -1, 50);
    CGRect tie[] = {CGRectMake(0, 0, 100, 100), CGRectMake(100, 0, 100, 50)};
    CGRect tie_reversed[] = {tie[1], tie[0]};
    p = fievel_display_constrained_destination(CGPointMake(90, 40), CGPointMake(20, 20), tie, 2);
    expect_point(p, 99, 60);
    p = fievel_display_constrained_destination(CGPointMake(90, 40), CGPointMake(20, 20), tie_reversed, 2);
    expect_point(p, 99, 60);
    p = fievel_display_constrained_destination(CGPointMake(90, 40), CGPointMake(10, 10), tie, 2);
    expect_point(p, 99, 50);
    p = fievel_display_constrained_destination(p, CGPointMake(10, 10), tie, 2);
    expect_point(p, 99, 60);
    p = fievel_display_constrained_destination(CGPointMake(90, 40), CGPointMake(10, 10), tie_reversed, 2);
    expect_point(p, 99, 50);
    p = fievel_display_constrained_destination(p, CGPointMake(10, 10), tie_reversed, 2);
    expect_point(p, 99, 60);
    double enter, exit;
    assert(fievel_segment_display_interval(CGPointMake(99.3, 49.3 + 4e-15), CGPointMake(1.4, 1.4), tie[1], &enter, &exit));
    assert(enter <= exit);
    CGRect step_right[] = {CGRectMake(0, 0, 100, 100), CGRectMake(100, 50, 100, 100)};
    p = fievel_display_constrained_destination(CGPointMake(90, 30), CGPointMake(30, 30), step_right, 2);
    expect_point(p, 120, 60);
    CGRect step_down[] = {CGRectMake(0, 0, 100, 100), CGRectMake(50, 100, 100, 100)};
    p = fievel_display_constrained_destination(CGPointMake(30, 90), CGPointMake(30, 30), step_down, 2);
    expect_point(p, 60, 120);
    p = fievel_display_constrained_destination(CGPointMake(0, 5), CGPointMake(0, -15), displays, 2);
    expect_point(p, 0, 0);
    CGRect fractional_seam[] = {CGRectMake(-100.5, 0, 100, 100), CGRectMake(-0.5, 0, 100, 100), CGRectMake(-0.5, 100, 100, 100)};
    p = fievel_display_constrained_destination(CGPointMake(-0.38, 99.2), CGPointMake(-5.74, 18.58), fractional_seam, 3);
    expect_point(p, -6.12, 99.2);
    p = fievel_display_constrained_destination(CGPointMake(5, 5), CGPointMake(10, 10), NULL, 0);
    expect_point(p, 15, 15);

    puts("macOS edge geometry passed");
}
