//! egui adapter for `cunning_syntax`.

use cunning_syntax::{HighlightKind, LanguageId, Span, kinds};
use egui::{self, Color32, FontId, TextBuffer, TextFormat, text::LayoutJob};

pub fn layouter(lang: LanguageId) -> impl FnMut(&egui::Ui, &dyn TextBuffer, f32) -> std::sync::Arc<egui::Galley> {
    move |ui, buf, wrap_width| {
        let font_id: FontId = egui::TextStyle::Monospace.resolve(ui.style());
        let base = TextFormat::simple(font_id.clone(), ui.visuals().text_color());
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap_width;
        job.break_on_newline = true;
        build_job(&mut job, &base, lang, buf.as_str());
        ui.fonts_mut(|f| f.layout_job(job))
    }
}

pub fn build_job(job: &mut LayoutJob, base: &TextFormat, lang: LanguageId, text: &str) {
    let syn = cunning_syntax::highlight(lang, text);
    let mut line_idx = 0usize;
    for raw in text.split_inclusive('\n') {
        let (line, nl) = raw.strip_suffix('\n').map(|x| (x, "\n")).unwrap_or((raw, ""));
        let spans = syn.as_ref().and_then(|s| s.lines.get(line_idx));
        append_line(job, base, line, spans);
        if !nl.is_empty() { job.append(nl, 0.0, base.clone()); }
        line_idx += 1;
    }
    if !text.ends_with('\n') && text.is_empty() {
        job.append("", 0.0, base.clone());
    }
}

fn append_line(job: &mut LayoutJob, base: &TextFormat, line: &str, spans: Option<&Vec<Span>>) {
    let Some(spans) = spans else { job.append(line, 0.0, base.clone()); return; };
    if spans.is_empty() { job.append(line, 0.0, base.clone()); return; }

    let mut pos = 0usize;
    for sp in spans.iter() {
        let ss = clamp_back(line, sp.range.start.min(line.len()));
        let ee = clamp_fwd(line, sp.range.end.min(line.len()));
        if ee <= ss { continue; }
        if ss > pos {
            let a = clamp_back(line, pos);
            if ss > a { job.append(line.get(a..ss).unwrap_or(""), 0.0, base.clone()); }
        }
        let mut fmt = base.clone();
        fmt.color = color_for_kind(sp.kind);
        job.append(line.get(ss..ee).unwrap_or(""), 0.0, fmt);
        pos = ee.max(pos);
    }
    if pos < line.len() { job.append(line.get(clamp_back(line, pos)..).unwrap_or(""), 0.0, base.clone()); }
}

fn color_for_kind(k: HighlightKind) -> Color32 {
    if k == kinds::COMMENT { return Color32::from_gray(150); }
    if k == kinds::KEYWORD { return Color32::from_rgb(86, 156, 214); }
    if k == kinds::STRING { return Color32::from_rgb(206, 145, 120); }
    if k == kinds::TYPE { return Color32::from_rgb(78, 201, 176); }
    if k == kinds::FUNCTION { return Color32::from_rgb(220, 220, 170); }
    if k == kinds::CONSTANT { return Color32::from_rgb(79, 193, 255); }
    if k == kinds::NUMBER { return Color32::from_rgb(181, 206, 168); }
    if k == kinds::OPERATOR { return Color32::from_rgb(212, 212, 212); }
    if k == kinds::PUNCTUATION { return Color32::from_rgb(212, 212, 212); }
    if k == kinds::VARIABLE { return Color32::from_rgb(220, 220, 220); }
    Color32::from_rgb(220, 220, 220)
}

fn clamp_back(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) { i -= 1; }
    i
}

fn clamp_fwd(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) { i += 1; }
    i
}

