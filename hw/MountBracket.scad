hole_dir_outer=8.5;
hole_dir_inner=4;
$fn=100;

height=17;

difference () {
union () {
    
translate([-5,-5,0]) cube([60,70,5]);
translate([0, 0,0]) pillar(hole_dir_outer,hole_dir_inner,height);
translate([50, 0,0]) pillar(hole_dir_outer,hole_dir_inner,height);
translate([0, 60,0]) pillar(hole_dir_outer,hole_dir_inner,height);
translate([50, 60,0]) pillar(hole_dir_outer,hole_dir_inner,height);
}

translate([0, 0,-0.01]) cylinder(d=4, h=8);
translate([50, 0,-0.01]) cylinder(d=4, h=8);
translate([0, 60,-0.01]) cylinder(d=4, h=8);
translate([50, 60,-0.01]) cylinder(d=4, h=8);
}

module pillar(outer, inner, height) {
    difference() {
        cylinder(d=outer, h=height);
        translate([0,0,height/2 + 0.1]) cylinder(d=inner, h=height/2);
    }
}
