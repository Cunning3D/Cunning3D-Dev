use super::super::structures::ProfileSpacing;
use super::math::{PRO_CIRCLE_R, PRO_LINE_R, PRO_SQUARE_IN_R, PRO_SQUARE_R};
use std::collections::HashMap;
use std::sync::RwLock;

// ProfileSpacing 缓存：避免重复计算相同 (seg, pro_r) 的曲线采样
type SpacingCacheKey = (usize, u32); // (seg, pro_r as bits)
static SPACING_CACHE: RwLock<Option<HashMap<SpacingCacheKey, ProfileSpacing>>> = RwLock::new(None);

fn cache_key(seg: usize, r: f32) -> SpacingCacheKey {
    (seg, r.to_bits())
}

fn get_cached(key: SpacingCacheKey) -> Option<ProfileSpacing> {
    SPACING_CACHE.read().ok()?.as_ref()?.get(&key).cloned()
}

fn set_cached(key: SpacingCacheKey, val: ProfileSpacing) {
    if let Ok(mut guard) = SPACING_CACHE.write() {
        let cache = guard.get_or_insert_with(HashMap::new);
        if cache.len() > 64 {
            cache.clear();
        } // 限制缓存大小
        cache.insert(key, val);
    }
}

fn superellipse_co(x: f64, r: f32, rbig: bool) -> f64 {
    if rbig {
        (1.0 - x.powf(r as f64)).powf(1.0 / r as f64)
    } else {
        1.0 - (1.0 - (1.0 - x).powf(r as f64)).powf(1.0 / r as f64)
    }
}

fn find_superellipse_chord_endpoint(x0: f64, dtarget: f64, r: f32, rbig: bool) -> f64 {
    let y0 = superellipse_co(x0, r, rbig);
    let tol = 1e-13;
    let maxiter = 10;
    let mut xmin = (x0 + (std::f64::consts::SQRT_2 / 2.0) * dtarget).min(1.0);
    let mut xmax = (x0 + dtarget).min(1.0);
    let ymin = superellipse_co(xmin, r, rbig);
    let ymax = superellipse_co(xmax, r, rbig);
    let dist = |x: f64, y: f64| ((x - x0).powi(2) + (y - y0).powi(2)).sqrt();
    let mut dmaxerr = dist(xmax, ymax) - dtarget;
    let mut dminerr = dist(xmin, ymin) - dtarget;
    let mut xnew = xmax - dmaxerr * (xmax - xmin) / (dmaxerr - dminerr);
    let mut lastupdated_upper = true;
    for _ in 0..maxiter {
        let ynew = superellipse_co(xnew, r, rbig);
        let dnewerr = dist(xnew, ynew) - dtarget;
        if dnewerr.abs() < tol {
            break;
        }
        if dnewerr < 0.0 {
            xmin = xnew;
            dminerr = dnewerr;
            xnew = if !lastupdated_upper {
                (dmaxerr / 2.0 * xmin - dminerr * xmax) / (dmaxerr / 2.0 - dminerr)
            } else {
                xmax - dmaxerr * (xmax - xmin) / (dmaxerr - dminerr)
            };
            lastupdated_upper = false;
        } else {
            xmax = xnew;
            dmaxerr = dnewerr;
            xnew = if lastupdated_upper {
                (dmaxerr * xmin - dminerr / 2.0 * xmax) / (dmaxerr - dminerr / 2.0)
            } else {
                xmax - dmaxerr * (xmax - xmin) / (dmaxerr - dminerr)
            };
            lastupdated_upper = true;
        }
    }
    xnew
}

fn find_even_superellipse_chords_general(seg: usize, r: f32, xvals: &mut [f64], yvals: &mut [f64]) {
    let smoothitermax = 10;
    let error_tol = 1e-7;
    let imax = (seg + 1) / 2 - 1;
    let seg_odd = (seg & 1) == 1;
    let (rbig, mx) = if r > 1.0 {
        (true, (0.5f64).powf(1.0 / r as f64))
    } else {
        (false, 1.0 - (0.5f64).powf(1.0 / r as f64))
    };
    for i in 0..=imax {
        xvals[i] = (i as f64) * mx / seg as f64 * 2.0;
        yvals[i] = superellipse_co(xvals[i], r, rbig);
    }
    yvals[0] = 1.0;
    for _ in 0..smoothitermax {
        let mut sum = 0.0;
        let mut dmin: f64 = 2.0;
        let mut dmax: f64 = 0.0;
        for i in 0..imax {
            let d = ((xvals[i + 1] - xvals[i]).powi(2) + (yvals[i + 1] - yvals[i]).powi(2)).sqrt();
            sum += d;
            dmax = dmax.max(d);
            dmin = dmin.min(d);
        }
        let davg = if seg_odd {
            sum += (std::f64::consts::SQRT_2 / 2.0) * (yvals[imax] - xvals[imax]);
            sum / (imax as f64 + 0.5)
        } else {
            sum += ((xvals[imax] - mx).powi(2) + (yvals[imax] - mx).powi(2)).sqrt();
            sum / (imax as f64 + 1.0)
        };
        if dmax - davg <= error_tol && dmin - davg >= -error_tol {
            break;
        }
        for i in 1..=imax {
            xvals[i] = find_superellipse_chord_endpoint(xvals[i - 1], davg, r, rbig);
            yvals[i] = superellipse_co(xvals[i], r, rbig);
        }
    }
    if !seg_odd {
        xvals[imax + 1] = mx;
        yvals[imax + 1] = mx;
    }
    for i in (imax + 1)..=seg {
        yvals[i] = xvals[seg - i];
        xvals[i] = yvals[seg - i];
    }
    if !rbig {
        for i in 0..=seg {
            let t = xvals[i];
            xvals[i] = 1.0 - yvals[i];
            yvals[i] = 1.0 - t;
        }
    }
}

fn find_even_superellipse_chords(seg: usize, r: f32, xvals: &mut [f64], yvals: &mut [f64]) {
    let seg_odd = (seg & 1) == 1;
    let seg2 = seg / 2;
    if (r - PRO_LINE_R).abs() < 1e-6 {
        for i in 0..=seg {
            xvals[i] = i as f64 / seg as f64;
            yvals[i] = 1.0 - i as f64 / seg as f64;
        }
        return;
    }
    if (r - PRO_CIRCLE_R).abs() < 1e-6 {
        let temp = std::f64::consts::FRAC_PI_2 / seg as f64;
        for i in 0..=seg {
            xvals[i] = (i as f64 * temp).sin();
            yvals[i] = (i as f64 * temp).cos();
        }
        return;
    }
    if (r - PRO_SQUARE_IN_R).abs() < 1e-6 {
        if !seg_odd {
            for i in 0..=seg2 {
                xvals[i] = 0.0;
                yvals[i] = 1.0 - i as f64 / seg2 as f64;
                xvals[seg - i] = yvals[i];
                yvals[seg - i] = xvals[i];
            }
        } else {
            let temp = 1.0 / (seg2 as f64 + std::f64::consts::SQRT_2 / 2.0);
            for i in 0..=seg2 {
                xvals[i] = 0.0;
                yvals[i] = 1.0 - (i as f64) * temp;
                xvals[seg - i] = yvals[i];
                yvals[seg - i] = xvals[i];
            }
        }
        return;
    }
    if (r - PRO_SQUARE_R).abs() < 1e-3 {
        if !seg_odd {
            for i in 0..=seg2 {
                xvals[i] = i as f64 / seg2 as f64;
                yvals[i] = 1.0;
                xvals[seg - i] = yvals[i];
                yvals[seg - i] = xvals[i];
            }
        } else {
            let temp = 1.0 / (seg2 as f64 + std::f64::consts::SQRT_2 / 2.0);
            for i in 0..=seg2 {
                xvals[i] = (i as f64) * temp;
                yvals[i] = 1.0;
                xvals[seg - i] = yvals[i];
                yvals[seg - i] = xvals[i];
            }
        }
        return;
    }
    find_even_superellipse_chords_general(seg, r, xvals, yvals);
}

/// 构建 profile 采样点（带缓存，避免重复计算）
pub fn build_profile_spacing(seg: usize, r: f32) -> ProfileSpacing {
    if seg <= 1 {
        return ProfileSpacing {
            seg_2: 0,
            ..Default::default()
        };
    }

    // 检查缓存
    let key = cache_key(seg, r);
    if let Some(cached) = get_cached(key) {
        return cached;
    }

    // 计算
    let seg_2 = seg.next_power_of_two().max(4);
    let mut xvals = vec![0.0f64; seg + 1];
    let mut yvals = vec![0.0f64; seg + 1];
    find_even_superellipse_chords(seg, r, &mut xvals, &mut yvals);
    let (xvals_2, yvals_2) = if seg_2 == seg {
        (xvals.clone(), yvals.clone())
    } else {
        let mut x2 = vec![0.0f64; seg_2 + 1];
        let mut y2 = vec![0.0f64; seg_2 + 1];
        find_even_superellipse_chords(seg_2, r, &mut x2, &mut y2);
        (x2, y2)
    };
    let fullness = if (r - PRO_CIRCLE_R).abs() < 1e-6 {
        const T: [f32; 11] = [
            0.0, 0.559, 0.642, 0.551, 0.646, 0.624, 0.646, 0.619, 0.647, 0.639, 0.647,
        ];
        if seg == 0 {
            0.0
        } else if seg <= 11 {
            T[seg - 1]
        } else {
            0.647
        }
    } else {
        0.0
    };

    let result = ProfileSpacing {
        xvals,
        yvals,
        xvals_2,
        yvals_2,
        seg_2,
        fullness,
    };
    set_cached(key, result.clone());
    result
}
