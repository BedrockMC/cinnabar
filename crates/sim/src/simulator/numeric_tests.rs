use super::{Vec3, apply_relative_movement};

#[test]
fn ordinary_steering_uses_native_float_trigonometry_and_product_order() {
    // Lens 1.26.50.26 0x99cc8b0; R:d/DefaultMoveSystems.cpp:32-57.
    for (yaw, strafe, speed, initial, expected) in [
        (33.333, 0.0, 0.02, [0.0, 0.0], [0xbc30_75d4, 0x3c86_262c]),
        (5.625, 0.98, 0.1, [0.07, -0.03], [0x3e08_a452, 0x3d41_bebe]),
        (90.0, 0.0, 0.02, [0.0, 0.0], [0xbca0_902e, 0xb06b_7ff2]),
    ] {
        let mut velocity = Vec3::new(initial[0], 0.0, initial[1]);
        apply_relative_movement(&mut velocity, strafe, 0.98, yaw, speed);
        assert_eq!((velocity.x as f32).to_bits(), expected[0], "yaw {yaw}");
        assert_eq!((velocity.z as f32).to_bits(), expected[1], "yaw {yaw}");
        assert_eq!(velocity.x, f64::from(velocity.x as f32));
        assert_eq!(velocity.z, f64::from(velocity.z as f32));
    }
}
