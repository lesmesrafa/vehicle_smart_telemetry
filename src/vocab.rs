//! The vocabulary of the knowledge graph: every IRI the programs name, already written as an
//! N-Triples term. Standard vocabularies are used where one exists: RDF and RDFS for types and
//! labels, SOSA (the W3C sensor ontology) for what a signal observes and for observations, and
//! XSD for the datatypes of numbers. The rest is the project's own, under `jedge:`.

// Where the names of this car live. The builder adds the name of each thing after the `#`.
pub const SIGNALS: &str = "https://example.org/jedge/hyundai-ccan/signal#";
pub const MESSAGES: &str = "https://example.org/jedge/hyundai-ccan/message#";
pub const ECUS: &str = "https://example.org/jedge/hyundai-ccan/ecu#";
/// Where quantities and equations live. They do not depend on the car.
pub const JEDGE: &str = "https://example.org/jedge/voc#";

pub const TYPE: &str = "<http://www.w3.org/1999/02/22-rdf-syntax-ns#type>";
pub const LABEL: &str = "<http://www.w3.org/2000/01/rdf-schema#label>";
pub const SUB_PROPERTY_OF: &str = "<http://www.w3.org/2000/01/rdf-schema#subPropertyOf>";

// SOSA: a signal observes a quantity, and an observation is one value of it.
pub const OBSERVABLE_PROPERTY: &str = "<http://www.w3.org/ns/sosa/ObservableProperty>";
pub const OBSERVES: &str = "<http://www.w3.org/ns/sosa/observes>";
pub const OBSERVATION: &str = "<http://www.w3.org/ns/sosa/Observation>";
pub const MADE_BY_SENSOR: &str = "<http://www.w3.org/ns/sosa/madeBySensor>";
pub const OBSERVED_PROPERTY: &str = "<http://www.w3.org/ns/sosa/observedProperty>";
pub const HAS_SIMPLE_RESULT: &str = "<http://www.w3.org/ns/sosa/hasSimpleResult>";

// Classes of the project.
pub const SIGNAL: &str = "<https://example.org/jedge/voc#Signal>";
pub const MESSAGE: &str = "<https://example.org/jedge/voc#Message>";
pub const ECU: &str = "<https://example.org/jedge/voc#Ecu>";
pub const QUANTITY: &str = "<https://example.org/jedge/voc#Quantity>";
pub const EQUATION: &str = "<https://example.org/jedge/voc#Equation>";
pub const WINDOW: &str = "<https://example.org/jedge/voc#Window>";

// Properties of the fixed part: what the DBC says about signals, messages and ECUs.
pub const NAME: &str = "<https://example.org/jedge/voc#name>";
pub const CAN_ID: &str = "<https://example.org/jedge/voc#canId>";
pub const IN_MESSAGE: &str = "<https://example.org/jedge/voc#inMessage>";
pub const SENT_BY: &str = "<https://example.org/jedge/voc#sentBy>";
pub const RECEIVED_BY: &str = "<https://example.org/jedge/voc#receivedBy>";
pub const UNIT: &str = "<https://example.org/jedge/voc#unit>";
pub const RESOLUTION: &str = "<https://example.org/jedge/voc#resolution>";
pub const EVENT_THRESHOLD: &str = "<https://example.org/jedge/voc#eventThreshold>";
// What the table of default quantities says of a signal nobody has checked: the quantity it
// is taken to observe, and how sure that reading is (high, medium or low).
pub const OBSERVES_BY_DEFAULT: &str = "<https://example.org/jedge/voc#observesByDefault>";
pub const CONFIDENCE: &str = "<https://example.org/jedge/voc#confidence>";
// Properties of an equation: the kind of relation and the quantities it ties together.
pub const FUNCTION: &str = "<https://example.org/jedge/voc#function>";
pub const OVER: &str = "<https://example.org/jedge/voc#over>";

// Properties of the live part: a window of a question and what each signal did in it.
pub const QUESTION: &str = "<https://example.org/jedge/voc#question>";
pub const ENDS_AT: &str = "<https://example.org/jedge/voc#endsAt>";
pub const SECONDS: &str = "<https://example.org/jedge/voc#seconds>";
pub const IN_WINDOW: &str = "<https://example.org/jedge/voc#inWindow>";
pub const STREAM_TIME: &str = "<https://example.org/jedge/voc#streamTime>";
pub const SAMPLES: &str = "<https://example.org/jedge/voc#samples>";
pub const WINDOW_FIRST: &str = "<https://example.org/jedge/voc#windowFirst>";
pub const WINDOW_MIN: &str = "<https://example.org/jedge/voc#windowMin>";
pub const WINDOW_MAX: &str = "<https://example.org/jedge/voc#windowMax>";
pub const WINDOW_MEAN: &str = "<https://example.org/jedge/voc#windowMean>";

// What follows the closing quote of a literal that is a number, or a text in English.
pub const DOUBLE: &str = "^^<http://www.w3.org/2001/XMLSchema#double>";
pub const DECIMAL: &str = "^^<http://www.w3.org/2001/XMLSchema#decimal>";
pub const INTEGER: &str = "^^<http://www.w3.org/2001/XMLSchema#integer>";
pub const ENGLISH: &str = "@en";
