$fn=50;

base_thickness = 7.5;
pillar_height = 35;

pillar_dia = 10;
piller_inner_hole_dia = 4;

module bracket1 () {

    pillar_1_loc = [ 65.65/2, 117.05/2,0];
    pillar_2_loc = [-65.65/2,117.05/2,0];
    pillar_3_loc = [-30.3/2, 135.75/2,0];
    pillar_4_loc = [30.3/2, 135.75/2,0];

    difference() {
        union() {
            //Add on the pillars
            translate([25,117.05/2,0]) difference() {
                cylinder(h=pillar_height,d=pillar_dia);
                translate([0,0,pillar_height-5])cylinder(h=10,d=piller_inner_hole_dia);
            }
            
            translate([-25,117.05/2,0]) difference() {
                cylinder(h=pillar_height,d=pillar_dia);
                translate([0,0,pillar_height-5])cylinder(h=10,d=piller_inner_hole_dia);
            }
            
            hull() {
                translate(pillar_1_loc) cylinder(d=10,h=base_thickness);
                translate(pillar_2_loc) cylinder(d=10,h=base_thickness);
                translate(pillar_3_loc) cylinder(d=10,h=base_thickness);
                translate(pillar_4_loc) cylinder(d=10,h=base_thickness);
            };
        }
        //Drill out the four mounting post holes
        translate([0,0,-0.01]) translate(pillar_1_loc) cylinder(d=7, h=5);
        translate([0,0,-0.01]) translate(pillar_2_loc) cylinder(d=7, h=5);
        translate([0,0,-0.01]) translate(pillar_3_loc) cylinder(d=7, h=5);
        translate([0,0,-0.01]) translate(pillar_4_loc) cylinder(d=7, h=5);
        
        //screw holes
        translate([0,0,-0.01]) translate(pillar_1_loc) cylinder(d=2.5, h=50);
        translate([0,0,-0.01]) translate(pillar_2_loc) cylinder(d=2.5, h=50);
        translate([0,0,-0.01]) translate(pillar_3_loc) cylinder(d=2.5, h=50);
        translate([0,0,-0.01]) translate(pillar_4_loc) cylinder(d=2.5, h=50);
    }
}


module bracket2() {
        
    pillar_5_loc = [ 65.65/2,-117.05/2,0];
    pillar_6_loc = [-65.65/2,-117.05/2,0];
    pillar_7_loc = [30.3/2, -135.75/2,0];
    pillar_8_loc = [-30.3/2, -135.75/2,0];

    difference() {
        union() {
            //Add on the pillars
            translate([25,117.05/2 - 105,0]) difference() {
                cylinder(h=pillar_height,d=pillar_dia);
                translate([0,0,pillar_height-5])cylinder(h=10,d=piller_inner_hole_dia);
            }
            
            translate([-25,117.05/2 - 105,0]) difference() {
                cylinder(h=pillar_height,d=pillar_dia);
                translate([0,0,pillar_height-5])cylinder(h=10,d=piller_inner_hole_dia);
            }
            
            hull() {
                //point to merge the and pillars together
                translate([0,15,0]) translate(pillar_5_loc) cylinder(d=8,h=base_thickness);
                translate([0,15,0]) translate(pillar_6_loc) cylinder(d=8,h=base_thickness);

                
                translate(pillar_5_loc) cylinder(d=10,h=base_thickness);
                translate(pillar_6_loc) cylinder(d=10,h=base_thickness);
                translate(pillar_7_loc) cylinder(d=10,h=base_thickness);
                translate(pillar_8_loc) cylinder(d=10,h=base_thickness);
            };
        }
        //Drill out the four mounting post holes
        translate([0,0,-0.01]) translate(pillar_5_loc) cylinder(d=7, h=5);
        translate([0,0,-0.01]) translate(pillar_6_loc) cylinder(d=7, h=5);
        translate([0,0,-0.01]) translate(pillar_7_loc) cylinder(d=7, h=5);
        translate([0,0,-0.01]) translate(pillar_8_loc) cylinder(d=7, h=5);
        
        //screw holes
        translate([0,0,-0.01]) translate(pillar_5_loc) cylinder(d=2.5, h=50);
        translate([0,0,-0.01]) translate(pillar_6_loc) cylinder(d=2.5, h=50);
        translate([0,0,-0.01]) translate(pillar_7_loc) cylinder(d=2.5, h=50);
        translate([0,0,-0.01]) translate(pillar_8_loc) cylinder(d=2.5, h=50);
    }
}


bracket1();
bracket2();
