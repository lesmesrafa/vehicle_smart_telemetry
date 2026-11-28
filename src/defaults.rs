//! The table of default quantities: what a signal is taken to carry until someone checks it.
//!
//! A DBC names its signals and gives their units, but it does not say that `TEMP_ENG` is the
//! temperature of the coolant. For the signals whose quantity was read from their name, unit and
//! range, that reading is kept in a table: one signal per line, with tabs between the columns.
//! The builder of the graph reads it here.

use std::fs;
use std::io;

/// One row of the table.
#[derive(Debug, PartialEq)]
pub struct Row {
    /// The signal, as MESSAGE.SIGNAL.
    pub signal: String,
    /// The quantity it carries. Empty when the signal is not a quantity: a flag, an index.
    pub quantity: String,
    /// How a person reads that quantity, or what the signal is when it has none.
    pub label: String,
    pub unit: String,
    /// How sure the reading is: high, medium or low.
    pub confidence: String,
    /// Whether the reading has been checked against values of the signal.
    pub verified: bool,
}

/// The columns read here. A table may have more, in any order.
const COLUMNS: [&str; 6] = ["signal", "quantity", "label", "unit", "confidence", "checked_with_values"];

/// Reads the table from a file.
pub fn load(path: &str) -> io::Result<Vec<Row>> {
    let text = fs::read_to_string(path).map_err(|e| crate::with_name(path, e))?;
    parse(&text).map_err(|e| io::Error::other(format!("{path}: {e}")))
}

/// Reads the table from its text. The first line names the columns.
pub fn parse(text: &str) -> Result<Vec<Row>, String> {
    let mut lines = text.lines().enumerate().filter(|(_, line)| !line.trim().is_empty());
    let (_, header) = lines.next().ok_or("the table is empty")?;
    let names: Vec<&str> = header.split('\t').map(str::trim).collect();
    let mut at = [0usize; 6];
    for (i, column) in COLUMNS.iter().enumerate() {
        at[i] = names.iter().position(|name| name == column).ok_or(format!("the table has no column '{column}'"))?;
    }

    let mut rows: Vec<Row> = Vec::new();
    for (i, line) in lines {
        let cells: Vec<&str> = line.split('\t').map(str::trim).collect();
        // A line may end before its last columns when they are empty.
        let cell = |column: usize| cells.get(at[column]).copied().unwrap_or("").to_string();
        let row = Row {
            signal: cell(0),
            quantity: cell(1),
            label: cell(2),
            unit: cell(3),
            confidence: cell(4),
            verified: cell(5) == "yes",
        };
        let wrong = |what: String| format!("line {}: {what}", i + 1);
        let named = row.signal.split_once('.').is_some_and(|(m, s)| !m.is_empty() && !s.is_empty());
        if !named {
            return Err(wrong(format!("'{}' is not a signal written as MESSAGE.SIGNAL", row.signal)));
        }
        // The quantity becomes part of an IRI of the graph.
        if !row.quantity.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return Err(wrong(format!("'{}' is not a quantity: lower case letters, digits and _", row.quantity)));
        }
        if row.label.is_empty() {
            return Err(wrong(format!("{} has no label", row.signal)));
        }
        if !["high", "medium", "low"].contains(&row.confidence.as_str()) {
            return Err(wrong(format!("the confidence of {} is '{}': high, medium or low", row.signal, row.confidence)));
        }
        if rows.iter().any(|other| other.signal == row.signal) {
            return Err(wrong(format!("{} appears twice", row.signal)));
        }
        // One quantity has one unit, whichever signal carries it.
        let same = |other: &&Row| !row.quantity.is_empty() && other.quantity == row.quantity && other.unit != row.unit;
        if let Some(other) = rows.iter().find(same) {
            return Err(wrong(format!(
                "{} is in '{}' for {} and in '{}' for {}",
                row.quantity, row.unit, row.signal, other.unit, other.signal
            )));
        }
        rows.push(row);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "signal\tunit\tquantity\tlabel\tconfidence\tnote\tchecked_with_values\n\
        EMS11.VS\tkm/h\tvehicle_speed\tvehicle speed\thigh\t1 km/h steps\tyes\n\
        EMS12.TEMP_ENG\tdegC\tcoolant_temperature\tengine coolant temperature\tmedium\n\
        CLU11.CF_Clu_ParityBit1\t\t\tparity bit of the cluster message\thigh\t\t\n";

    #[test]
    fn the_columns_are_found_by_name() {
        let rows = parse(TABLE).unwrap();
        assert_eq!(rows.len(), 3);
        let first = Row {
            signal: "EMS11.VS".to_string(),
            quantity: "vehicle_speed".to_string(),
            label: "vehicle speed".to_string(),
            unit: "km/h".to_string(),
            confidence: "high".to_string(),
            verified: true,
        };
        assert_eq!(rows[0], first);
        // The second line ends before its last two columns, and the third signal has no quantity.
        assert!(!rows[1].verified);
        assert_eq!((rows[2].quantity.as_str(), rows[2].unit.as_str()), ("", ""));
    }

    #[test]
    fn a_table_that_cannot_be_right_is_refused() {
        let refused = |text: &str| parse(text).err().unwrap();
        assert!(refused("").contains("empty"));
        assert!(refused(&TABLE.replace("confidence", "sure")).contains("no column 'confidence'"));
        assert!(refused(&TABLE.replace("medium", "maybe")).contains("line 3"));
        assert!(refused(&TABLE.replace("EMS12.TEMP_ENG", "EMS11.VS")).contains("twice"));
        assert!(refused(&TABLE.replace("EMS11.VS", "VS")).contains("MESSAGE.SIGNAL"));
        assert!(refused(&TABLE.replace("coolant_temperature", "Coolant Temperature")).contains("not a quantity"));
        // One quantity in two units.
        assert!(refused(&TABLE.replace("coolant_temperature", "vehicle_speed")).contains("'degC' for EMS12.TEMP_ENG"));
    }
}
