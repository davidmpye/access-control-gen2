
hole_dir_outer=7;
hole_dir_inner=4;
//Takes M3 threaded brass inserts
$fn=100;

height=12;
pillar(hole_dir_outer,hole_dir_inner,height);

module pillar(outer, inner, height) {
    difference() {
        cylinder(d=outer, h=height);
        translate([0,0,-0.1]) cylinder(d=inner, h=height + 0.2);
    }
}
