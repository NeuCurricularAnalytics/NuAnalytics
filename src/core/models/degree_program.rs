//! Degree program structure for YAML deserialization
//!
//! A degree program consists of:
//! - Degree metadata (name, institution, requirements, etc.)
//! - Requirements mapping (ID -> Requirement)
//! - Courses mapping (subject+number -> Course)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::degree::Requirement;
use super::{Course, Degree};

/// A complete degree program with metadata, requirements, and courses
///
/// This is the top-level structure deserialized from YAML degree files.
/// It combines:
/// - A `Degree` for program metadata
/// - A requirements map for degree requirements
/// - A courses map for course definitions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DegreeProgram {
    /// Degree metadata (id, institution, credits, GPA, etc.)
    pub degree: Degree,

    /// Requirements mapping (requirement ID -> Requirement definition)
    pub requirements: HashMap<String, Requirement>,

    /// Courses mapping (course key like "ICS111" -> Course)
    pub courses: HashMap<String, Course>,
}
