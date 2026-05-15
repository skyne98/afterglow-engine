use bevy::prelude::*;

/// Earth surface gravity acceleration in m/s².
pub const GRAVITY_ACCELERATION: f32 = 9.81;
pub const GRAVITY_VEC: Vec3 = Vec3::new(0.0, -9.81, 0.0);

/// Weight (force in Newtons) of a mass under Earth gravity.
pub const fn weight_kg(kg: f32) -> f32 {
    kg * GRAVITY_ACCELERATION
}

/// === Mass wrappers (all convert to f32 kilograms) ===

pub struct Kilograms(pub f32);
impl From<Kilograms> for f32 {
    fn from(v: Kilograms) -> Self {
        v.0
    }
}

pub struct Grams(pub f32);
impl From<Grams> for f32 {
    fn from(v: Grams) -> Self {
        v.0 * 0.001
    }
}

/// === Length wrappers (all convert to f32 meters) ===

pub struct Meters(pub f32);
impl From<Meters> for f32 {
    fn from(v: Meters) -> Self {
        v.0
    }
}

pub struct Centimeters(pub f32);
impl From<Centimeters> for f32 {
    fn from(v: Centimeters) -> Self {
        v.0 * 0.01
    }
}

pub struct Inches(pub f32);
impl From<Inches> for f32 {
    fn from(v: Inches) -> Self {
        v.0 * 0.0254
    }
}

pub struct Feet(pub f32);
impl From<Feet> for f32 {
    fn from(v: Feet) -> Self {
        v.0 * 0.3048
    }
}

/// === Density (kg/m³) with real-world material presets ===

#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Density(pub f32);

impl Density {
    pub const STEEL: Self = Self(7800.0);
    pub const IRON: Self = Self(7870.0);
    pub const STAINLESS_STEEL: Self = Self(8000.0);
    pub const ALUMINUM: Self = Self(2700.0);
    pub const COPPER: Self = Self(8960.0);
    pub const BRASS: Self = Self(8500.0);
    pub const BRONZE: Self = Self(8800.0);
    pub const GOLD: Self = Self(19300.0);
    pub const SILVER: Self = Self(10490.0);
    pub const LEAD: Self = Self(11340.0);
    pub const TITANIUM: Self = Self(4500.0);
    pub const WOOD_PINE: Self = Self(500.0);
    pub const WOOD_OAK: Self = Self(750.0);
    pub const WOOD_BALSA: Self = Self(160.0);
    pub const CONCRETE: Self = Self(2400.0);
    pub const STONE_GRANITE: Self = Self(2700.0);
    pub const STONE_LIMESTONE: Self = Self(2300.0);
    pub const BRICK: Self = Self(2000.0);
    pub const GLASS: Self = Self(2500.0);
    pub const RUBBER: Self = Self(1100.0);
    pub const ICE: Self = Self(917.0);
    pub const WATER: Self = Self(1000.0);
    pub const HUMAN: Self = Self(985.0);
    pub const SOIL: Self = Self(1500.0);
    pub const SAND: Self = Self(1600.0);
    pub const DEFAULT: Self = Self(1000.0);
}

impl From<Density> for f32 {
    fn from(v: Density) -> Self {
        v.0
    }
}

/// Compute the weight (Newtons) of `kg` under Earth gravity.
pub const fn newtons(kg: f32) -> f32 {
    kg * GRAVITY_ACCELERATION
}

/// Volume of a cuboid in cubic meters.
pub const fn cuboid_volume(w: f32, h: f32, d: f32) -> f32 {
    w * h * d
}

/// Volume of a sphere in cubic meters.
pub fn sphere_volume(radius: f32) -> f32 {
    (4.0 / 3.0) * std::f32::consts::PI * radius * radius * radius
}

/// Volume of a cylinder in cubic meters.
pub fn cylinder_volume(radius: f32, height: f32) -> f32 {
    std::f32::consts::PI * radius * radius * height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gravity_is_earth_standard() {
        assert!((GRAVITY_ACCELERATION - 9.81).abs() < 0.01);
    }

    #[test]
    fn weight_of_100kg_is_981_newtons() {
        let w = weight_kg(100.0);
        assert!((w - 981.0).abs() < 1.0);
    }

    #[test]
    fn kilograms_convert_to_f32() {
        let v: f32 = Kilograms(50.0).into();
        assert!((v - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn grams_convert_to_kg() {
        let v: f32 = Grams(500.0).into();
        assert!((v - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn centimeters_convert_to_meters() {
        let v: f32 = Centimeters(250.0).into();
        assert!((v - 2.5).abs() < f32::EPSILON);
    }

    #[test]
    fn inches_convert_to_meters() {
        let v: f32 = Inches(12.0).into();
        assert!((v - 0.3048).abs() < 0.0001);
    }

    #[test]
    fn feet_convert_to_meters() {
        let v: f32 = Feet(3.0).into();
        assert!((v - 0.9144).abs() < 0.0001);
    }

    #[test]
    fn density_material_presets_have_distinct_values() {
        assert!(Density::STEEL.0 > Density::WOOD_PINE.0);
        assert!(Density::WOOD_OAK.0 > Density::WOOD_BALSA.0);
        assert!(Density::GOLD.0 > Density::IRON.0);
        assert!(Density::WATER.0 > Density::ICE.0);
        assert!(Density::ALUMINUM.0 < Density::COPPER.0);
    }

    #[test]
    fn steel_density_is_correct() {
        assert!((Density::STEEL.0 - 7800.0).abs() < f32::EPSILON);
    }

    #[test]
    fn water_density_is_correct() {
        assert!((Density::WATER.0 - 1000.0).abs() < f32::EPSILON);
    }

    #[test]
    fn wood_pine_is_lighter_than_oak() {
        assert!(Density::WOOD_PINE.0 < Density::WOOD_OAK.0);
    }

    #[test]
    fn human_density_close_to_water() {
        let ratio = Density::HUMAN.0 / Density::WATER.0;
        assert!((ratio - 0.985).abs() < 0.01);
    }

    #[test]
    fn default_density_equals_water() {
        assert!((Density::DEFAULT.0 - Density::WATER.0).abs() < f32::EPSILON);
    }

    #[test]
    fn cuboid_volume_1m_cube_is_1() {
        let v = cuboid_volume(1.0, 1.0, 1.0);
        assert!((v - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn cuboid_volume_0_5m_cube_is_0_125() {
        let v = cuboid_volume(0.5, 0.5, 0.5);
        assert!((v - 0.125).abs() < f32::EPSILON);
    }

    #[test]
    fn sphere_volume_1m_radius() {
        let v = sphere_volume(1.0);
        let expected = (4.0 / 3.0) * std::f32::consts::PI;
        assert!((v - expected).abs() < 0.0001);
    }

    #[test]
    fn cylinder_volume_1m_radius_2m_height() {
        let v = cylinder_volume(1.0, 2.0);
        let expected = std::f32::consts::PI * 2.0;
        assert!((v - expected).abs() < 0.0001);
    }

    #[test]
    fn steel_cube_mass_1m_is_7800kg() {
        let volume = cuboid_volume(1.0, 1.0, 1.0);
        let mass = volume * Density::STEEL.0;
        assert!((mass - 7800.0).abs() < f32::EPSILON);
    }

    #[test]
    fn steel_cube_weight_1m() {
        let volume = cuboid_volume(1.0, 1.0, 1.0);
        let mass = volume * Density::STEEL.0;
        let weight = weight_kg(mass);
        assert!((weight - 7800.0 * 9.81).abs() < 1.0);
    }

    #[test]
    fn concrete_cube_0_5m_mass() {
        let volume = cuboid_volume(0.5, 0.5, 0.5);
        let mass = volume * Density::CONCRETE.0;
        assert!((mass - 300.0).abs() < 0.1);
    }

    #[test]
    fn iron_is_denser_than_aluminum() {
        assert!(Density::IRON.0 > Density::ALUMINUM.0);
    }

    #[test]
    fn gold_is_densest_common_metal() {
        assert!(Density::GOLD.0 > Density::LEAD.0);
        assert!(Density::GOLD.0 > Density::COPPER.0);
        assert!(Density::GOLD.0 > Density::STEEL.0);
    }
}
