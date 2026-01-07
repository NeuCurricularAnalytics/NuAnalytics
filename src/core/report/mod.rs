//! Report generation module for curriculum analysis
//!
//! This module provides functionality to generate curriculum reports in various formats
//! (Markdown, HTML, PDF) with visualizations of the curriculum graph and term scheduling.

pub mod formats;
pub mod term_scheduler;
pub mod visualization;

use crate::core::metrics::CurriculumMetrics;
use crate::core::metrics_export::CurriculumSummary;
use crate::core::models::{Degree, Plan, School, DAG};
use std::error::Error;
use std::path::Path;

pub use formats::{HtmlReporter, MarkdownReporter, PdfReporter, ReportFormat};
pub use term_scheduler::{SchedulerConfig, TermPlan, TermScheduler};
pub use visualization::MermaidGenerator;

/// Data context for report generation
///
/// This struct aggregates all data needed to render a curriculum report,
/// providing a single source of truth for templates.
#[derive(Debug, Clone)]
pub struct ReportContext<'a> {
    /// School containing course catalog
    pub school: &'a School,
    /// Curriculum plan being reported
    pub plan: &'a Plan,
    /// Associated degree (if found)
    pub degree: Option<&'a Degree>,
    /// Computed metrics for courses
    pub metrics: &'a CurriculumMetrics,
    /// Summary statistics
    pub summary: &'a CurriculumSummary,
    /// Prerequisite DAG
    pub dag: &'a DAG,
    /// Term-by-term course schedule
    pub term_plan: &'a TermPlan,
}

impl<'a> ReportContext<'a> {
    /// Create a new report context
    #[must_use]
    pub const fn new(
        school: &'a School,
        plan: &'a Plan,
        degree: Option<&'a Degree>,
        metrics: &'a CurriculumMetrics,
        summary: &'a CurriculumSummary,
        dag: &'a DAG,
        term_plan: &'a TermPlan,
    ) -> Self {
        Self {
            school,
            plan,
            degree,
            metrics,
            summary,
            dag,
            term_plan,
        }
    }

    /// Get the institution name
    #[must_use]
    pub fn institution_name(&self) -> &str {
        self.plan
            .institution
            .as_deref()
            .unwrap_or(&self.school.name)
    }

    /// Get the degree name or a default
    #[must_use]
    pub fn degree_name(&self) -> String {
        self.degree
            .map_or_else(|| self.plan.degree_id.clone(), Degree::id)
    }

    /// Get the system type (semester/quarter)
    #[must_use]
    pub fn system_type(&self) -> &str {
        self.degree.map_or("semester", |d| d.system_type.as_str())
    }

    /// Get the CIP code
    #[must_use]
    pub fn cip_code(&self) -> &str {
        self.degree.map_or("", |d| d.cip_code.as_str())
    }

    /// Calculate total credit hours
    #[must_use]
    pub fn total_credits(&self) -> f32 {
        self.plan
            .courses
            .iter()
            .filter_map(|key| self.school.get_course(key))
            .map(|c| c.credit_hours)
            .sum()
    }

    /// Get course count
    #[must_use]
    pub const fn course_count(&self) -> usize {
        self.plan.courses.len()
    }

    /// Calculate the number of years from the term plan
    #[allow(clippy::cast_precision_loss)]
    #[must_use]
    pub fn years(&self) -> f32 {
        let terms_used = self.term_plan.terms_used();
        let terms_per_year = if self.term_plan.is_quarter_system {
            3.0 // quarters per year
        } else {
            2.0 // semesters per year
        };
        (terms_used as f32 / terms_per_year).ceil()
    }
}

/// Trait for report generators
pub trait ReportGenerator {
    /// Generate a report to a file
    ///
    /// # Errors
    /// Returns an error if report generation or file writing fails
    fn generate(&self, ctx: &ReportContext, output_path: &Path) -> Result<(), Box<dyn Error>>;

    /// Generate report content as a string
    ///
    /// # Errors
    /// Returns an error if report generation fails
    fn render(&self, ctx: &ReportContext) -> Result<String, Box<dyn Error>>;
}
