use crate::*;

pub const LADDER_STEP_HEIGHT: f32 = 24.0;
pub const LADDER_STEP_WIDTH: f32 = 40.0;

const FLOAT_LADDER_POWERS: [i32; 7] = [2, 1, 0, -1, -2, -3, -4];
const INT_LADDER_POWERS: [i32; 3] = [2, 1, 0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LadderState {
    pub start_pos: Pos2,
    pub initial_value: f64,
    pub current_power: i32, // 10^power
    pub active: bool,
    pub widget_id: Id,
    pub is_integral: bool,
    pub base_index: i32,
}

impl LadderState {
    pub fn new(start_pos: Pos2, initial_value: f64, id: Id, is_integral: bool) -> Self {
        let (current_power, base_index) = if is_integral {
            let base_power = 0;
            let index = INT_LADDER_POWERS
                .iter()
                .position(|&p| p == base_power)
                .unwrap_or(0) as i32;
            (base_power, index)
        } else {
            let base_power = -1;
            let index = FLOAT_LADDER_POWERS
                .iter()
                .position(|&p| p == base_power)
                .unwrap_or(0) as i32;
            (base_power, index)
        };

        Self {
            start_pos,
            initial_value,
            current_power,
            active: true,
            widget_id: id,
            is_integral,
            base_index,
        }
    }

    pub fn ladder_powers(&self) -> &'static [i32] {
        if self.is_integral {
            &INT_LADDER_POWERS
        } else {
            &FLOAT_LADDER_POWERS
        }
    }
}

pub fn show_ladder_menu(ctx: &Context, state: &LadderState) {
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("ladder_menu")));
    
    let powers = state.ladder_powers();

    for (index, power) in powers.iter().enumerate() {
        let relative_index = index as i32 - state.base_index;
        let center_y = state.start_pos.y + (relative_index as f32) * LADDER_STEP_HEIGHT;
        let rect = Rect::from_center_size(
            pos2(state.start_pos.x, center_y),
            vec2(LADDER_STEP_WIDTH, LADDER_STEP_HEIGHT - 1.0),
        );

        let is_active = *power == state.current_power;

        let bg_color = if is_active {
            Color32::from_rgb(200, 140, 40)
        } else {
            Color32::from_black_alpha(200)
        };

        painter.rect_filled(rect, 2.0, bg_color);
        painter.rect_stroke(rect, 2.0, Stroke::new(1.0, Color32::from_gray(100)), StrokeKind::Middle);

        let val = 10.0f64.powi(*power);
        let text = if *power >= 0 {
            format!("{:.0}", val)
        } else {
            let precision = power.abs() as usize;
            let s = format!("{:.1$}", val, precision);
            s.replace("0.", ".")
        };

        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::monospace(12.0),
            Color32::WHITE,
        );
    }
}

/// Bounding rectangle of the ladder menu in screen space.
pub fn ladder_bounds(state: &LadderState) -> Rect {
    let powers = state.ladder_powers();
    if powers.is_empty() {
        return Rect::from_center_size(state.start_pos, Vec2::ZERO);
    }

    let first_center_y = state.start_pos.y + (0i32 - state.base_index) as f32 * LADDER_STEP_HEIGHT;
    let top = first_center_y - LADDER_STEP_HEIGHT * 0.5;
    let total_height = powers.len() as f32 * LADDER_STEP_HEIGHT;

    Rect::from_min_size(
        pos2(state.start_pos.x - LADDER_STEP_WIDTH * 0.5, top),
        vec2(LADDER_STEP_WIDTH, total_height),
    )
}
