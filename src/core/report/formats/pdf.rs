//! PDF report generator
//!
//! Generates curriculum reports directly in PDF format using the printpdf library.
//! Creates a clean, professional PDF layout with sections for:
//! - Header with plan/institution information
//! - Summary metrics
//! - Term schedule
//! - Course metrics table

use crate::core::report::{ReportContext, ReportGenerator};
use printpdf::{BuiltinFont, Mm, PdfDocument, PdfDocumentReference, PdfLayerIndex, PdfPageIndex};
use std::error::Error;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// PDF report generator using printpdf for native PDF creation
pub struct PdfReporter;

impl PdfReporter {
    /// Create a new PDF reporter
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Create the PDF document with proper setup
    fn create_document(ctx: &ReportContext) -> (PdfDocumentReference, PdfPageIndex, PdfLayerIndex) {
        let title = format!("{} - Curriculum Report", ctx.plan.name);
        let (doc, page1, layer1) = PdfDocument::new(&title, Mm(210.0), Mm(297.0), "Layer 1");
        (doc, page1, layer1)
    }

    /// Add header section with plan information
    fn add_header(
        doc: &PdfDocumentReference,
        page: PdfPageIndex,
        layer: PdfLayerIndex,
        ctx: &ReportContext,
        y_pos: &mut f32,
    ) {
        let current_layer = doc.get_page(page).get_layer(layer);
        let font = doc.add_builtin_font(BuiltinFont::HelveticaBold).unwrap();
        let font_regular = doc.add_builtin_font(BuiltinFont::Helvetica).unwrap();

        // Title
        current_layer.use_text(&ctx.plan.name, 24.0, Mm(20.0), Mm(280.0), &font);
        *y_pos -= 10.0;

        // Institution and degree info
        let info_text = format!(
            "{} - {} {}",
            ctx.institution_name(),
            ctx.degree.map_or("Unknown", |d| d.degree_type.as_str()),
            ctx.degree_name()
        );
        current_layer.use_text(&info_text, 12.0, Mm(20.0), Mm(270.0), &font_regular);
        *y_pos -= 15.0;
    }

    /// Add summary metrics section
    fn add_summary(
        doc: &PdfDocumentReference,
        page: PdfPageIndex,
        layer: PdfLayerIndex,
        ctx: &ReportContext,
        y_pos: &mut f32,
    ) {
        let current_layer = doc.get_page(page).get_layer(layer);
        let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold).unwrap();
        let font = doc.add_builtin_font(BuiltinFont::Helvetica).unwrap();

        // Section title
        current_layer.use_text("Summary Metrics", 16.0, Mm(20.0), Mm(*y_pos), &font_bold);
        *y_pos -= 8.0;

        // Metrics
        let metrics = vec![
            ("Total Courses:", ctx.course_count().to_string()),
            ("Total Credits:", format!("{:.1}", ctx.total_credits())),
            (
                "Total Complexity:",
                ctx.summary.total_complexity.to_string(),
            ),
            (
                "Longest Delay:",
                format!(
                    "{} ({})",
                    ctx.summary.longest_delay, ctx.summary.longest_delay_course
                ),
            ),
            (
                "Highest Centrality:",
                format!(
                    "{} ({})",
                    ctx.summary.highest_centrality, ctx.summary.highest_centrality_course
                ),
            ),
            ("Terms Used:", ctx.term_plan.terms.len().to_string()),
        ];

        for (label, value) in metrics {
            current_layer.use_text(label, 11.0, Mm(20.0), Mm(*y_pos), &font_bold);
            current_layer.use_text(&value, 11.0, Mm(80.0), Mm(*y_pos), &font);
            *y_pos -= 6.0;
        }
        *y_pos -= 5.0;
    }

    /// Add term schedule section
    fn add_term_schedule(
        doc: &PdfDocumentReference,
        page: PdfPageIndex,
        layer: PdfLayerIndex,
        ctx: &ReportContext,
        y_pos: &mut f32,
    ) {
        let current_layer = doc.get_page(page).get_layer(layer);
        let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold).unwrap();
        let font = doc.add_builtin_font(BuiltinFont::Helvetica).unwrap();

        // Section title
        current_layer.use_text("Term Schedule", 16.0, Mm(20.0), Mm(*y_pos), &font_bold);
        *y_pos -= 8.0;

        // Terms
        for (idx, term) in ctx.term_plan.terms.iter().enumerate() {
            if *y_pos < 30.0 {
                // Add new page if needed
                break;
            }

            let term_name = format!("Term {} ({} credits):", idx + 1, term.total_credits);
            current_layer.use_text(&term_name, 11.0, Mm(20.0), Mm(*y_pos), &font_bold);
            *y_pos -= 6.0;

            for course_id in &term.courses {
                if let Some(course) = ctx.school.get_course(course_id) {
                    let course_text = format!(
                        "  {} - {} ({} credits)",
                        course_id, course.name, course.credit_hours
                    );
                    current_layer.use_text(&course_text, 9.0, Mm(25.0), Mm(*y_pos), &font);
                    *y_pos -= 5.0;

                    if *y_pos < 30.0 {
                        break;
                    }
                }
            }
            *y_pos -= 3.0;
        }
    }

    /// Add course metrics table
    fn add_course_metrics(
        doc: &PdfDocumentReference,
        page: PdfPageIndex,
        layer: PdfLayerIndex,
        ctx: &ReportContext,
        y_pos: &mut f32,
    ) {
        let current_layer = doc.get_page(page).get_layer(layer);
        let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold).unwrap();
        let font = doc.add_builtin_font(BuiltinFont::Helvetica).unwrap();

        if *y_pos < 60.0 {
            return; // Not enough space
        }

        // Section title
        current_layer.use_text("Course Metrics", 16.0, Mm(20.0), Mm(*y_pos), &font_bold);
        *y_pos -= 8.0;

        // Table header
        current_layer.use_text("Course", 9.0, Mm(20.0), Mm(*y_pos), &font_bold);
        current_layer.use_text("Complexity", 9.0, Mm(70.0), Mm(*y_pos), &font_bold);
        current_layer.use_text("Delay", 9.0, Mm(110.0), Mm(*y_pos), &font_bold);
        current_layer.use_text("Centrality", 9.0, Mm(140.0), Mm(*y_pos), &font_bold);
        *y_pos -= 6.0;

        // Course rows (limit to avoid overflow)
        for (course_id, metrics) in ctx.metrics.iter().take(30) {
            if *y_pos < 30.0 {
                break;
            }

            current_layer.use_text(course_id, 8.0, Mm(20.0), Mm(*y_pos), &font);
            current_layer.use_text(
                metrics.complexity.to_string(),
                8.0,
                Mm(75.0),
                Mm(*y_pos),
                &font,
            );
            current_layer.use_text(metrics.delay.to_string(), 8.0, Mm(115.0), Mm(*y_pos), &font);
            current_layer.use_text(
                metrics.centrality.to_string(),
                8.0,
                Mm(145.0),
                Mm(*y_pos),
                &font,
            );
            *y_pos -= 5.0;
        }
    }
}

impl Default for PdfReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ReportGenerator for PdfReporter {
    /// Generate PDF report directly
    ///
    /// Creates a native PDF with formatted sections including header, summary,
    /// term schedule, and course metrics.
    fn generate(&self, ctx: &ReportContext, output_path: &Path) -> Result<(), Box<dyn Error>> {
        // Create PDF document
        let (doc, page1, layer1) = Self::create_document(ctx);

        // Track vertical position (starting from top)
        let mut y_pos = 280.0;

        // Add sections
        Self::add_header(&doc, page1, layer1, ctx, &mut y_pos);
        Self::add_summary(&doc, page1, layer1, ctx, &mut y_pos);
        Self::add_term_schedule(&doc, page1, layer1, ctx, &mut y_pos);
        Self::add_course_metrics(&doc, page1, layer1, ctx, &mut y_pos);

        // Save PDF
        let file = File::create(output_path)?;
        let mut writer = BufWriter::new(file);
        doc.save(&mut writer)?;

        Ok(())
    }

    /// Render method for consistency with other reporters
    ///
    /// PDF generation requires binary output, so this returns a placeholder message.
    fn render(&self, _ctx: &ReportContext) -> Result<String, Box<dyn Error>> {
        Ok(String::from(
            "PDF reports are binary and must be generated directly to file.",
        ))
    }
}
