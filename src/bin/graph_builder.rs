//! Builds the fixed part of the knowledge graph from the DBC and writes it as N-Triples.
//!
//! From the DBC come the signals, their messages and the ECUs that send and receive them. From
//! this file come the two things a DBC does not say: which physical quantity a signal measures,
//! and which equations tie those quantities together. For the signals this file does not name,
//! the table of default quantities gives the quantity each one is taken to carry.

use std::collections::BTreeSet;
use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write};
use vehicle_smart_telemetry::dbc::Dbc;
use vehicle_smart_telemetry::defaults;
use vehicle_smart_telemetry::ntriples::{iri, literal};
use vehicle_smart_telemetry::options::{fail, Args};
use vehicle_smart_telemetry::{vocab, with_name};

const HELP: &str = "\
graph_builder: writes the fixed part of the knowledge graph, from the DBC, as N-Triples.

use:  graph_builder
      graph_builder --dbc hyundai_2015_ccan.dbc --defaults default_quantities.tsv --out signal_graph.nt

  --dbc FILE        DBC file (default hyundai_2015_ccan.dbc)
  --defaults FILE   table of default quantities, for the signals with none here (see default_quantities.tsv)
  --out FILE        where to write the graph (default signal_graph.nt)";
const WITH_VALUE: [&str; 3] = ["--dbc", "--defaults", "--out"];
const WITHOUT_VALUE: [&str; 0] = [];

/// A quantity a question can name: its identifier, how a person reads it, its unit, the signal
/// of this car that measures it and the change of that signal worth calling an event.
type Quantity = (&'static str, &'static str, &'static str, &'static str, f64);

const QUANTITIES: [Quantity; 20] = [
    ("vehicle_speed", "vehicle speed", "km/h", "CLU11.CF_Clu_Vanz", 0.5),
    ("wheel_speed_fl", "speed of the front left wheel", "km/h", "WHL_SPD11.WHL_SPD_FL", 0.5),
    ("wheel_speed_fr", "speed of the front right wheel", "km/h", "WHL_SPD11.WHL_SPD_FR", 0.5),
    ("wheel_speed_rl", "speed of the rear left wheel", "km/h", "WHL_SPD11.WHL_SPD_RL", 0.5),
    ("wheel_speed_rr", "speed of the rear right wheel", "km/h", "WHL_SPD11.WHL_SPD_RR", 0.5),
    ("longitudinal_acceleration", "longitudinal acceleration, negative when braking", "m/s^2", "ESP12.LONG_ACCEL", 0.2),
    ("lateral_acceleration", "lateral acceleration", "m/s^2", "ESP12.LAT_ACCEL", 0.2),
    ("yaw_rate", "yaw rate", "deg/s", "ESP12.YAW_RATE", 0.5),
    ("brake_pressure", "pressure of the brake circuit", "bar", "ESP12.CYL_PRES", 1.0),
    ("steering_wheel_angle", "steering wheel angle, positive to the left", "deg", "SAS11.SAS_Angle", 1.0),
    ("steering_wheel_speed", "turning speed of the steering wheel", "", "SAS11.SAS_Speed", 4.0),
    ("brake_active", "brake active flag", "", "EMS12.BRAKE_ACT", 1.0),
    ("accelerator_pedal", "position of the accelerator pedal", "%", "EMS12.PV_AV_CAN", 1.0),
    ("throttle", "throttle opening", "%", "EMS12.TPS", 1.0),
    ("engine_speed", "engine speed", "rpm", "EMS11.N", 50.0),
    ("engine_torque", "indicated engine torque", "%", "EMS11.TQI", 1.0),
    ("gear", "gear engaged", "", "TCU12.CUR_GR", 1.0),
    ("steering_column_torque", "torque on the steering column", "Nm", "MDPS12.CR_Mdps_StrColTq", 0.2),
    ("lead_vehicle_distance", "distance to the vehicle ahead", "m", "SCC11.ACC_ObjDist", 0.5),
    ("lead_vehicle_relative_speed", "relative speed of the vehicle ahead", "m/s", "SCC11.ACC_ObjRelSpd", 0.2),
];

/// An equation: its identifier, the kind of relation, how a person reads it and the quantities
/// it ties together.
type Equation = (&'static str, &'static str, &'static str, &'static [&'static str]);

const EQUATIONS: [Equation; 6] = [
    (
        "lateral_acceleration_eq",
        "product",
        "the lateral acceleration is the speed times the yaw rate",
        &["lateral_acceleration", "vehicle_speed", "yaw_rate"],
    ),
    (
        "longitudinal_acceleration_eq",
        "derivative",
        "the rate of change of the speed is the longitudinal acceleration, apart from the slope of the road",
        &["vehicle_speed", "longitudinal_acceleration"],
    ),
    (
        "wheel_speed_eq",
        "equality",
        "the vehicle speed is the mean of the four wheel speeds",
        &["vehicle_speed", "wheel_speed_fl", "wheel_speed_fr", "wheel_speed_rl", "wheel_speed_rr"],
    ),
    (
        "bicycle_eq",
        "bicycle",
        "the yaw rate follows from the speed and the steering wheel angle",
        &["yaw_rate", "vehicle_speed", "steering_wheel_angle"],
    ),
    (
        "differential_eq",
        "differential",
        "the yaw rate follows from the difference between the two rear wheel speeds",
        &["yaw_rate", "wheel_speed_rl", "wheel_speed_rr"],
    ),
    (
        "brake_eq",
        "implication",
        "with pressure in the brake circuit, the longitudinal acceleration is negative",
        &["brake_pressure", "longitudinal_acceleration"],
    ),
];

fn main() {
    if let Err(error) = run() {
        fail(&error.to_string());
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let a = Args::parse(HELP, &WITH_VALUE, &WITHOUT_VALUE);
    let dbc_path = a.text("--dbc", "hyundai_2015_ccan.dbc");
    let out_path = a.text("--out", "signal_graph.nt");
    let dbc = Dbc::load(&dbc_path)?;

    // The table of default quantities. Each row must name a signal of the DBC that has no
    // quantity in this file, and a quantity of this file keeps its unit.
    let table = match a.optional("--defaults") {
        Some(path) => defaults::load(path)?,
        None => Vec::new(),
    };
    for row in &table {
        let (message, signal) = row.signal.split_once('.').unwrap_or_default();
        if dbc.by_name(message).and_then(|m| m.position(signal)).is_none() {
            return Err(format!("{}: the DBC has no such signal", row.signal).into());
        }
        if QUANTITIES.iter().any(|q| q.3 == row.signal) {
            return Err(format!("{} already has a quantity in graph_builder", row.signal).into());
        }
        if let Some(q) = QUANTITIES.iter().find(|q| q.0 == row.quantity && q.2 != row.unit) {
            let (name, unit) = (q.0, q.2);
            return Err(format!("{}: {name} is in '{unit}' here and in '{}' in the table", row.signal, row.unit).into());
        }
    }

    let mut out = BufWriter::new(File::create(&out_path).map_err(|e| with_name(&out_path, e))?);
    let mut triples = 0u64;
    let mut add = |subject: &str, predicate: &str, object: &str| -> std::io::Result<()> {
        triples += 1;
        writeln!(out, "{subject} {predicate} {object} .")
    };
    let jedge = |name: &str| iri(&format!("{}{name}", vocab::JEDGE));
    let ecu = |name: &str| iri(&format!("{}{name}", vocab::ECUS));
    let double = |value: f64| literal(&value.to_string(), vocab::DOUBLE);

    // What the DBC knows: ECUs, messages and signals.
    let is_ecu = |name: &str| !name.is_empty() && name != "XXX" && name != "Vector__XXX";
    let mut ecus: BTreeSet<&str> = BTreeSet::new();
    let mut signals = 0usize;
    let mut measured: Vec<&str> = Vec::new();
    for message in &dbc.messages {
        let message_node = iri(&format!("{}{}", vocab::MESSAGES, message.name));
        add(&message_node, vocab::TYPE, vocab::MESSAGE)?;
        add(&message_node, vocab::NAME, &literal(&message.name, ""))?;
        add(&message_node, vocab::CAN_ID, &literal(&message.id.to_string(), vocab::INTEGER))?;
        if is_ecu(&message.sender) {
            ecus.insert(&message.sender);
            add(&message_node, vocab::SENT_BY, &ecu(&message.sender))?;
        }
        for signal in &message.signals {
            signals += 1;
            let key = format!("{}.{}", message.name, signal.name);
            let node = iri(&format!("{}{key}", vocab::SIGNALS));
            add(&node, vocab::TYPE, vocab::SIGNAL)?;
            add(&node, vocab::NAME, &literal(&signal.name, ""))?;
            add(&node, vocab::IN_MESSAGE, &message_node)?;
            add(&node, vocab::RESOLUTION, &double(signal.factor))?;
            if !signal.unit.is_empty() {
                add(&node, vocab::UNIT, &literal(&signal.unit, ""))?;
            }
            for (i, receiver) in signal.receivers.iter().enumerate() {
                // Some signals of this DBC list the same receiver twice.
                if is_ecu(receiver) && !signal.receivers[..i].contains(receiver) {
                    ecus.insert(receiver);
                    add(&node, vocab::RECEIVED_BY, &ecu(receiver))?;
                }
            }
            // What the DBC does not know: the quantity this signal measures.
            if let Some(&(quantity, _, _, _, threshold)) = QUANTITIES.iter().find(|q| q.3 == key) {
                measured.push(quantity);
                add(&node, vocab::OBSERVES, &jedge(quantity))?;
                add(&node, vocab::EVENT_THRESHOLD, &double(threshold))?;
            } else if let Some(row) = table.iter().find(|row| row.signal == key) {
                // Nobody named it here, and the table has a reading of it.
                if row.quantity.is_empty() {
                    add(&node, vocab::LABEL, &literal(&row.label, vocab::ENGLISH))?;
                } else if row.verified {
                    add(&node, vocab::OBSERVES, &jedge(&row.quantity))?;
                } else {
                    add(&node, vocab::OBSERVES_BY_DEFAULT, &jedge(&row.quantity))?;
                    add(&node, vocab::CONFIDENCE, &literal(&row.confidence, ""))?;
                }
            }
        }
    }
    for name in &ecus {
        add(&ecu(name), vocab::TYPE, vocab::ECU)?;
        add(&ecu(name), vocab::NAME, &literal(name, ""))?;
    }

    // Quantities: those of this file, then those only the table names, as its first row for each
    // one describes it.
    let mut quantities: Vec<(&str, &str, &str)> = QUANTITIES.iter().map(|q| (q.0, q.1, q.2)).collect();
    for row in &table {
        if !row.quantity.is_empty() && !quantities.iter().any(|q| q.0 == row.quantity) {
            quantities.push((&row.quantity, &row.label, &row.unit));
        }
    }
    for (quantity, label, unit) in &quantities {
        add(&jedge(quantity), vocab::TYPE, vocab::QUANTITY)?;
        add(&jedge(quantity), vocab::TYPE, vocab::OBSERVABLE_PROPERTY)?;
        add(&jedge(quantity), vocab::LABEL, &literal(label, vocab::ENGLISH))?;
        if !unit.is_empty() {
            add(&jedge(quantity), vocab::UNIT, &literal(unit, ""))?;
        }
    }
    if !table.is_empty() {
        // For a reader that applies RDFS, to observe by default is to observe.
        add(vocab::OBSERVES_BY_DEFAULT, vocab::SUB_PROPERTY_OF, vocab::OBSERVES)?;
    }
    // Equations.
    for (equation, function, label, over) in EQUATIONS {
        add(&jedge(equation), vocab::TYPE, vocab::EQUATION)?;
        add(&jedge(equation), vocab::FUNCTION, &literal(function, ""))?;
        add(&jedge(equation), vocab::LABEL, &literal(label, vocab::ENGLISH))?;
        for quantity in over {
            add(&jedge(equation), vocab::OVER, &jedge(quantity))?;
        }
    }
    out.flush()?;

    println!(
        "graph_builder: {dbc_path} defines {} messages, {signals} signals and {} ECUs",
        dbc.messages.len(),
        ecus.len()
    );
    println!(
        "{} quantities, {} of them measured by a signal of this DBC, and {} equations",
        QUANTITIES.len(),
        measured.len(),
        EQUATIONS.len()
    );
    let missing: Vec<&str> = QUANTITIES.iter().map(|q| q.0).filter(|q| !measured.contains(q)).collect();
    if !missing.is_empty() {
        println!("quantities with no signal in this DBC: {}", missing.join(", "));
    }
    if !table.is_empty() {
        let assumed = |confidence: &str| {
            table.iter().filter(|row| !row.quantity.is_empty() && !row.verified && row.confidence == confidence).count()
        };
        println!(
            "default quantities: {} more signals observe a quantity, {} of them checked with values",
            table.iter().filter(|row| !row.quantity.is_empty()).count(),
            table.iter().filter(|row| !row.quantity.is_empty() && row.verified).count()
        );
        println!(
            "by default, not checked: {} of high confidence, {} of medium and {} of low",
            assumed("high"),
            assumed("medium"),
            assumed("low")
        );
        println!(
            "{} new quantities; {} signals that are not a quantity are described",
            quantities.len() - QUANTITIES.len(),
            table.iter().filter(|row| row.quantity.is_empty()).count()
        );
    }
    println!("{triples} triples written to {out_path}");
    Ok(())
}
