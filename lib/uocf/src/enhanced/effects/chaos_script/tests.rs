use super::*;

/// A script in the style of the client files: CRLF, mixed indentation and case, comments, interleaved system
/// statements, a malformed number and a line that is no statement.
const SAMPLE: &str = "// leading comment\r\n\
particlesystem \"Sample\" 2.00 \r\n\
\r\n\
{\r\n\
   minemittime = 1.0\r\n\
   eventSequence \"first\"\r\n\
   {\r\n\
\ttype = Point\r\n\
\ttexture = \"spark.tga\"\r\n\
\tsourceBlendMode = ALPHA_SRCALPHA\r\n\
      burst = 1.5  //random(2,4)\r\n\
\tChaosLocation = XYZ(random(-1,1), 0,\r\n\
\t\trandom(5,6))\r\n\
\tcelAnimate = CelSetup(16,0.25,0.25,4,4,30,1)\r\n\
\tstayAtEmitter = TRUE\r\n\
//      initial rotation = XYZ(0,random(-180,180),0)\r\n\
\tinitial color = RGBA(0.3,0.1,0.0.5,0.2)\r\n\
\tfade so at random(0.3,0.5) color = RGBA(1,1,1,0.7)\r\n\
\tat 0.5 scale = 2\r\n\
\tfade so final scale = random(0.25,0.5)\r\n\
\tfinal alpha = 0\r\n\
\tthis is not a statement\r\n\
\tmysteryProperty = 3\r\n\
   }\r\n\
MinEmitTime = 2\r\n\
\teventsequence \"second\" {\r\n\
\t\ttype = AXIAL\r\n\
\t\tcollisionCheck = REFLECT\r\n\
\t}\r\n\
}";

fn sample() -> ChaosScript {
    ChaosScript::parse(SAMPLE).expect("the sample parses")
}

fn first_sequence(script: &mut ChaosScript) -> &mut ChaosSequence {
    script.sequence_mut("FIRST").expect("the first sequence")
}

#[test]
fn parses_the_sample() {
    let script = sample();
    assert_eq!(script.name, "Sample");
    assert_eq!(script.version, 2.0);
    assert_eq!(script.sequences().count(), 2);
    let first = script.sequences().next().unwrap();
    assert_eq!(first.name, "first");
    assert_eq!(
        first.property(ChaosProperty::Type).map(|statement| &statement.value),
        Some(&ChaosValue::ParticleType(ChaosParticleType::Point))
    );
    assert_eq!(
        first.property(ChaosProperty::ChaosLocation).map(|statement| &statement.value),
        Some(&ChaosValue::Vector([
            ChaosScalar::Random { min: -1.0, max: 1.0 },
            ChaosScalar::Number(0.0),
            ChaosScalar::Random { min: 5.0, max: 6.0 },
        ]))
    );
    assert_eq!(
        first.property(ChaosProperty::CelAnimate).map(|statement| &statement.value),
        Some(&ChaosValue::CelSetup(ChaosCelSetup {
            cel_count: 16.0,
            cel_u: 0.25,
            cel_v: 0.25,
            columns: 4.0,
            rows: 4.0,
            fps: 30.0,
            looping: 1.0,
        }))
    );
    let timings = first.statements().filter_map(|statement| statement.timing).collect::<Vec<_>>();
    assert_eq!(
        timings,
        [
            ChaosTiming::Initial,
            ChaosTiming::At { fade: true, time: ChaosScalar::Random { min: 0.3, max: 0.5 } },
            ChaosTiming::At { fade: false, time: ChaosScalar::Number(0.5) },
            ChaosTiming::Final { fade: true },
            ChaosTiming::Final { fade: false },
        ]
    );
    let initial_color = first.statements().find(|statement| statement.timing == Some(ChaosTiming::Initial)).unwrap();
    assert_eq!(
        initial_color.value,
        ChaosValue::Color([0.3, 0.1, 0.0, 0.2].map(ChaosScalar::Number)),
        "0.0.5 reads as 0.0"
    );
    assert!(first.items.iter().any(|item| matches!(item, ChaosItem::Raw(raw) if raw.text == "this is not a statement")));
    assert_eq!(script.referenced_files(), ["spark.tga"]);
    assert_eq!(initial_color.source_location(), Some(ChaosSourceLocation { line: 17, column: 2 }));
}

#[test]
fn lossless_output_is_the_parsed_text() {
    let script = sample();
    assert_eq!(script.to_lossless_string(), SAMPLE);
    assert_eq!(script.to_lossless_bytes().unwrap(), SAMPLE.as_bytes());
}

#[test]
fn canonical_output_reparses_to_an_equal_script() {
    let script = sample();
    let canonical = script.to_canonical_string();
    let reparsed = ChaosScript::parse(&canonical).unwrap();
    assert_eq!(reparsed, script);
    assert_eq!(reparsed.to_canonical_string(), canonical, "canonical output is stable");
    assert!(canonical.starts_with("particlesystem \"Sample\" 2\r\n{\r\n\tminEmitTime = 1\r\n"), "{canonical}");
    assert!(canonical.contains("\t\tchaosLocation = XYZ(random(-1,1),0,random(5,6))\r\n"), "{canonical}");
    assert!(canonical.contains("\t\tfade so at random(0.3,0.5) color = RGBA(1,1,1,0.7)\r\n"), "{canonical}");
}

#[test]
fn lossless_output_regenerates_only_edited_statements() {
    let mut script = sample();
    let first = first_sequence(&mut script);
    let burst = first
        .items
        .iter_mut()
        .find_map(|item| match item {
            ChaosItem::Statement(statement) if statement.key == ChaosKey::Known(ChaosProperty::Burst) => {
                Some(statement)
            }
            _ => None,
        })
        .unwrap();
    burst.value = ChaosValue::Scalar(ChaosScalar::Random { min: 1.0, max: 2.0 });
    let expected = SAMPLE.replace("burst = 1.5  //random(2,4)", "burst = random(1,2)  //random(2,4)");
    assert_eq!(script.to_lossless_string(), expected);
}

#[test]
fn lossless_output_indents_new_items_like_their_neighbours() {
    let mut script = sample();
    first_sequence(&mut script)
        .items
        .insert(1, ChaosItem::Statement(ChaosStatement::property(ChaosProperty::Lifetime, ChaosValue::Scalar(ChaosScalar::Number(0.95)))));
    let mut sequence = ChaosSequence::new("third");
    sequence.items.push(ChaosItem::Statement(ChaosStatement::property(
        ChaosProperty::Type,
        ChaosValue::ParticleType(ChaosParticleType::Mesh),
    )));
    script.items.push(ChaosItem::Sequence(sequence));
    let expected = SAMPLE
        .replace("\ttype = Point\r\n", "\ttype = Point\r\n\tlifetime = 0.95\r\n")
        .replace("\t}\r\n}", "\t}\r\n\teventsequence \"third\"\r\n\t{\r\n\t\ttype = MESH\r\n\t}\r\n}");
    let lossless = script.to_lossless_string();
    assert_eq!(lossless, expected);
    assert_eq!(ChaosScript::parse(&lossless).unwrap(), script);
}

#[test]
fn removing_an_item_drops_its_text() {
    let mut script = sample();
    first_sequence(&mut script).items.remove(0);
    assert_eq!(script.to_lossless_string(), SAMPLE.replace("\r\n\ttype = Point", ""));
}

#[test]
fn a_new_script_writes_canonical_text() {
    let mut script = ChaosScript::new("New", 1.0);
    script.items.push(ChaosItem::Statement(ChaosStatement::property(
        ChaosProperty::MinEmitTime,
        ChaosValue::Scalar(ChaosScalar::Number(1.0)),
    )));
    let mut sequence = ChaosSequence::new("only");
    sequence.items.push(ChaosItem::Statement(ChaosStatement::new(
        Some(ChaosTiming::At { fade: true, time: ChaosScalar::Number(0.5) }),
        ChaosKey::Known(ChaosProperty::Alpha),
        ChaosValue::Scalar(ChaosScalar::Number(0.25)),
    )));
    script.items.push(ChaosItem::Sequence(sequence));
    let expected = "particlesystem \"New\" 1\r\n{\r\n\tminEmitTime = 1\r\n\teventsequence \"only\"\r\n\t{\r\n\t\tfade so at 0.5 alpha = 0.25\r\n\t}\r\n}\r\n";
    assert_eq!(script.to_lossless_string(), expected);
    assert_eq!(script.to_canonical_string(), expected);
    assert_eq!(ChaosScript::parse(expected).unwrap(), script);
}

#[test]
fn lf_scripts_keep_lf_line_ends() {
    let text = "particlesystem \"Lf\" 1\n{\n\tminEmitTime = 1\n}\n";
    let script = ChaosScript::parse(text).unwrap();
    assert_eq!(script.to_lossless_string(), text);
    assert_eq!(script.to_canonical_string(), text);
}

#[test]
fn validation_reports_the_findings() {
    let script = sample();
    let existing = |name: &str| name != "spark.tga";
    let issues = script.validate(&ChaosValidationContext { file_exists: Some(&existing) });
    let kinds = issues.iter().map(|issue| &issue.kind).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            &ChaosIssueKind::MissingFile { property: ChaosProperty::Texture, name: "spark.tga".to_owned() },
            &ChaosIssueKind::MalformedNumber { text: "0.0.5".to_owned(), value: 0.0 },
            &ChaosIssueKind::UnparsedLine { text: "this is not a statement".to_owned() },
            &ChaosIssueKind::UnknownProperty { key: "mysteryProperty".to_owned() },
        ]
    );
    assert_eq!(issues[1].location, Some(ChaosSourceLocation { line: 17, column: 31 }), "the number's position");
    assert_eq!(issues[1].sequence.as_deref(), Some("first"));
    assert_eq!(issues[1].severity(), ChaosIssueSeverity::Warning);
    assert_eq!(issues[2].severity(), ChaosIssueSeverity::Error);
}

#[test]
fn validation_checks_values_and_event_order() {
    let mut script = ChaosScript::new("", 1.0);
    let mut sequence = ChaosSequence::new("s");
    let statements = [
        ChaosStatement::new(Some(ChaosTiming::Initial), ChaosKey::Known(ChaosProperty::Scale), ChaosValue::Scalar(ChaosScalar::Number(1.0))),
        ChaosStatement::property(ChaosProperty::Lifetime, ChaosValue::Bool(true)),
        ChaosStatement::new(Some(ChaosTiming::Final { fade: true }), ChaosKey::Known(ChaosProperty::Lifetime), ChaosValue::Scalar(ChaosScalar::Number(f64::NAN))),
        ChaosStatement::property(ChaosProperty::Texture, ChaosValue::FileName("a\"b".to_owned())),
    ];
    sequence.items.extend(statements.map(ChaosItem::Statement));
    script.items.push(ChaosItem::Sequence(sequence));
    script.items.push(ChaosItem::Statement(ChaosStatement::new(
        Some(ChaosTiming::Initial),
        ChaosKey::Known(ChaosProperty::Alpha),
        ChaosValue::Scalar(ChaosScalar::Number(1.0)),
    )));
    let issues = script.validate(&ChaosValidationContext::default());
    let kinds = issues.iter().map(|issue| issue.kind.clone()).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            ChaosIssueKind::EmptyName,
            ChaosIssueKind::UnexpectedValue { property: ChaosProperty::Lifetime, expected: ChaosValueKind::Scalar },
            ChaosIssueKind::PropertyAfterEvent { property: ChaosProperty::Lifetime },
            ChaosIssueKind::NonFiniteNumber,
            ChaosIssueKind::NotAnEvent { property: ChaosProperty::Lifetime },
            ChaosIssueKind::InvalidString { text: "a\"b".to_owned() },
            ChaosIssueKind::PropertyAfterEvent { property: ChaosProperty::Texture },
            ChaosIssueKind::EventOutsideSequence { property: ChaosProperty::Alpha },
        ]
    );
}

#[test]
fn syntax_errors_name_line_and_column() {
    let cases = [
        ("", "empty"),
        ("eventsequence \"x\" {}", "line 1, column 1"),
        ("particlesystem 1 {}", "line 1, column 16"),
        ("particlesystem \"x\"\r\n{}", "line 2, column 1"),
        ("particlesystem \"x\" 1\r\n{\r\n\teventsequence \"y\"\r\n\t{\r\n", "line 4, column 2"),
        ("particlesystem \"x\" 1 {} extra", "line 1, column 25"),
        ("particlesystem \"x\" 1 {\r\n\ttexture = \"open\r\n}", "line 2, column 12"),
        ("particlesystem \"x\" 1 { eventsequence \"a\" { eventsequence \"b\" {} } }", "line 1, column 44"),
    ];
    for (text, expected) in cases {
        let error = ChaosScript::parse(text).expect_err(text).to_string();
        assert!(error.contains(expected), "{text:?}: {error}");
    }
}

#[test]
fn latin1_bytes_survive_and_unicode_edits_are_rejected() {
    let bytes = b"particlesystem \"caf\xe9\" 1 {}";
    let mut script = ChaosScript::parse_bytes(bytes).unwrap();
    assert_eq!(script.name, "caf\u{e9}");
    assert_eq!(script.to_lossless_bytes().unwrap(), bytes);
    script.name = "\u{2603}".to_owned();
    assert!(script.to_lossless_bytes().is_err());
}
