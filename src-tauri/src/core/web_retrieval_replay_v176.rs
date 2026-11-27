use super::*;
use std::fs;
use std::path::PathBuf;

#[test]
#[ignore = "Explicit public webpage benchmark"]
fn capture_replay_v176() {
    let dir = PathBuf::from(
        env::var("SWARM_REPLAY_DIR").expect("missing replay directory")
    );

    let cases: Vec<(&str, &str, &str, Vec<&str>)> = vec![
        (
            "current_events",
            "Stockholm November 24-30 2026 jazz comedy markets museums music events",
            "Find five Stockholm activities for November 24-30 2026. \
             Include verified dates, venues, prices when available and \
             citations. Return valid JSON only. Do not invent details.",
            vec![
                "https://songkick.com/metro-areas/32252-sweden-stockholm/2026",
                "https://www.visitstockholm.com/see-do/attractions/biggest-upcoming-events/",
                "https://www.linkedin.com/company/stockholmfilmfestival",
                "https://visitstockholm.com/events/christmas-market-in-old-town/next",
            ],
        ),
        (
            "technical_lookup",
            "Python zoneinfo ambiguous daylight saving fold tzdata timezone database cache",
            "Explain Python ZoneInfo handling of ambiguous daylight-saving \
             times, timezone database availability, missing timezone data \
             and cache invalidation. Cite the documentation.",
            vec![
                "https://docs.python.org/3.13/library/zoneinfo.html",
                "https://docs.python.org/3.13/library/datetime.html",
            ],
        ),
        (
            "numerical_comparison",
            "PostgreSQL numeric precision scale negative scale rounding double precision",
            "Compare PostgreSQL numeric, real and double precision. \
             Explain exactness, precision, negative scale, storage and \
             rounding behavior using documented examples and citations.",
            vec![
                "https://www.postgresql.org/docs/17/datatype-numeric.html",
                "https://www.postgresql.org/docs/17/functions-math.html",
            ],
        ),
        (
            "detailed_research",
            "HTTP safe idempotent methods automatic retry cacheability Vary no-cache no-store",
            "Explain HTTP method safety, idempotency, automatic retries \
             and caching. Distinguish Vary, no-cache and no-store. \
             Preserve important exceptions and cite the specifications.",
            vec![
                "https://www.rfc-editor.org/rfc/rfc9110.html",
                "https://www.rfc-editor.org/rfc/rfc9111.html",
            ],
        ),
    ];

    let mut corpus = Vec::new();
    let mut accepted = 0;
    let mut rejected = 0;

    for (id, query, prompt, urls) in cases {
        println!("\nCASE={id}");
        let mut sources = Vec::new();

        for url in urls {
            let started = Instant::now();

            match fetch_native_web_text_v176(
                url, SCRAPE_MAX_BODY_BYTES_V1
            ) {
                Ok((final_url, raw, mime)) => {
                    let cleaned = clean_v1(&extract_web_text_v176(
                        &raw, mime.as_deref()
                    ));

                    let prefix = sanitize_web_text_v1(
                        &raw, mime.as_deref()
                    );

                    let selected =
                        crate::core::web_passage_selection_v176::select_passages(
                            &cleaned, query, "", MAX_EVIDENCE_CHARS_PER_SOURCE_V1
                        );

                    println!(
                        "SOURCE={} raw_bytes={} cleaned_chars={} \
                         prefix_chars={} selected_chars={} wall_ms={}",
                        url, raw.len(), cleaned.chars().count(),
                        prefix.chars().count(), selected.chars().count(),
                        started.elapsed().as_millis()
                    );

                    sources.push(serde_json::json!({
                        "url": url,
                        "final_url": final_url,
                        "mime": mime,
                        "raw_sha256": sha256_text_v1(&raw),
                        "raw": raw
                    }));

                    accepted += 1;
                }
                Err(reason) => {
                    println!("SOURCE_REJECTED={} reason={}", url, reason);
                    rejected += 1;
                }
            }
        }

        corpus.push(serde_json::json!({
            "id": id,
            "query": query,
            "prompt": prompt,
            "sources": sources
        }));
    }

    let path = dir.join("fixture.json");

    fs::write(
        &path, serde_json::to_vec(&corpus).unwrap()
    ).unwrap();

    println!("\nACCEPTED_SOURCES={accepted}");
    println!("REJECTED_SOURCES={rejected}");
    println!("REPLAY_FIXTURE={}", path.display());

    assert!(accepted > 0, "no usable benchmark sources");
}


#[test]
#[ignore = "Explicit isolated inference benchmark"]
fn inference_replay_v176() {
    let fixture = PathBuf::from(env::var("SWARM_REPLAY_FIXTURE").unwrap());
    let output = PathBuf::from(env::var("SWARM_REPLAY_OUTPUT").unwrap());
    let case_id = env::var("SWARM_REPLAY_CASE").unwrap();
    let variant = env::var("SWARM_REPLAY_VARIANT").unwrap();

    let corpus: Value =
        serde_json::from_slice(&fs::read(fixture).unwrap()).unwrap();

    let case = corpus.as_array().unwrap().iter()
        .find(|c| c["id"].as_str() == Some(case_id.as_str()))
        .unwrap();

    let prompt = case["prompt"].as_str().unwrap();
    let query = case["query"].as_str().unwrap();

    let llama = ProductionLlamaClient::new(
        "http://127.0.0.1:18082"
    ).unwrap();
    llama.health_check().unwrap();

    let started = Instant::now();
    let mut sources = Vec::new();

    for (i,row) in case["sources"].as_array().unwrap().iter().enumerate() {
        let raw = row["raw"].as_str().unwrap();
        let url = row["url"].as_str().unwrap();
        let mime = row["mime"].as_str();

        assert_eq!(
            sha256_text_v1(raw),
            row["raw_sha256"].as_str().unwrap()
        );

        let compactable = !matches!(
            variant.as_str(), "v174_prefix" | "a3_prefix"
        );

        let evidence = match variant.as_str() {
            "v174_prefix" => truncate_chars_v1(
                &legacy_markup_text_v176(raw), 12000
            ),
            "a3_prefix" => sanitize_web_text_v1(raw,mime),
            _ => crate::core::web_passage_selection_v176::select_passages(
                &clean_v1(&extract_web_text_v176(raw,mime)),
                query,"",12000
            )
        };

        sources.push(NodeWebSourceRecordV1 {
            selection_hint_v176: compactable.then(String::new),
            title: format!("Source {}",i+1),
            url: url.into(),
            domain: reqwest::Url::parse(url).unwrap()
                .host_str().unwrap_or("").into(),
            query: query.into(),
            evidence_sha256: sha256_text_v1(&evidence),
            evidence,
            broker_ticket: "offline-replay".into(),
            ticket_expires_at: 0,
            published_at: None,
            retrieved_at: None,
            provider: "offline-replay".into(),
        });
    }

    let mut report = serde_json::json!({
        "case": case_id,
        "variant": variant,
        "status": "pending"
    });

    let target = match variant.as_str() {
        "budget_8000" => Some(8000),
        "budget_4000" => Some(4000),
        "budget_2000" => Some(2000),
        _ => None
    };

    if let Some(target) = target {
        let original = llama.diagnostic_token_count_v176(prompt);

        let fitted = original
            .ok_or("tokenizer_unavailable".to_string())
            .and_then(|n| fit_evidence_budget_v176(
                &sources,n,target,|rows| {
                    llama.diagnostic_token_count_v176(
                        &build_grounded_prompt_v1(prompt,rows)
                    )
                }
            ));

        match fitted {
            Ok((selected,added,cap)) => {
                sources = selected;
                report["added_tokens"] = serde_json::json!(added);
                report["selected_cap"] = serde_json::json!(cap);
            }
            Err(reason) => {
                report["status"] = serde_json::json!("budget_failed");
                report["reason"] = serde_json::json!(reason);
                fs::write(&output,serde_json::to_vec_pretty(&report).unwrap()).unwrap();
                println!("REPLAY_RESULT={report}");
                return;
            }
        }
    }

    let grounded = build_grounded_prompt_v1(prompt,&sources);
    let preparation_ms = started.elapsed().as_millis() as u64;

    report["preparation_ms"] = serde_json::json!(preparation_ms);
    report["evidence_chars"] = serde_json::json!(
        sources.iter().map(|s| s.evidence.chars().count()).sum::<usize>()
    );

    report["generation_mode"] = serde_json::json!(
        crate::core::generation_policy::production_generation_settings_adaptive_v1(
            &grounded,None,None
        ).mode
    );

    match llama.execute_adaptive(&grounded,None,Some(900000)) {
        Ok(r) => {
            report["status"] = serde_json::json!("success");
            report["input_tokens"] = serde_json::json!(r.input_tokens);
            report["output_tokens"] = serde_json::json!(r.output_tokens);
            report["prompt_eval_ms"] = serde_json::json!(r.prompt_eval_ms);
            report["generation_ms"] = serde_json::json!(r.generation_ms);
            report["inference_ms"] = serde_json::json!(r.latency_ms);
            report["total_ms"] = serde_json::json!(started.elapsed().as_millis());
            report["cache_n"] = serde_json::json!(r.runtime_telemetry_v176.cache_n);
            report["finish_reason"] = serde_json::json!(r.finish_reason);
            report["token_count_method"] = serde_json::json!(r.token_count_method);
            println!("REPLAY_RESULT={report}");
            report["answer"] = serde_json::json!(r.ai_output);
        }
        Err(reason) => {
            report["status"] = serde_json::json!("inference_failed");
            report["reason"] = serde_json::json!(reason);
            println!("REPLAY_RESULT={report}");
        }
    }

    report["grounded_prompt"] = serde_json::json!(grounded);
    report["sources"] = serde_json::to_value(&sources).unwrap();

    fs::write(&output,serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}


fn coverage_words_v176(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 2 && !matches!(
            w.as_str(),
            "the" | "and" | "for" | "with" | "from" |
            "of" | "to" | "in" | "on" | "is" | "or" |
            "an" | "how" | "what" | "explain" | "compare" |
            "find" | "include" | "cite" | "return" |
            "valid" | "only" | "documentation"
        ))
        .collect()
}

fn coverage_blocks_v176(text: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut count = 0;
    let mut prev = ' ';

    for (i,c) in text.char_indices() {
        count += 1;

        let boundary = c == '\n' ||
            (c.is_whitespace() && matches!(prev,'.' | '!' | '?'));

        if (count >= 350 && boundary)
            || (count >= 900 && c.is_whitespace())
            || count >= 1200
        {
            let end = i + c.len_utf8();
            let block = text[start..end].trim();

            if !block.is_empty() { result.push(block); }

            start = end;
            count = 0;
        }
        prev = c;
    }

    if start < text.len() {
        let block = text[start..].trim();
        if !block.is_empty() { result.push(block); }
    }

    result
}

#[test]
#[ignore = "Explicit frozen-corpus coverage experiment"]
fn coverage_replay_v176() {
    let fixture = PathBuf::from(
        env::var("SWARM_REPLAY_FIXTURE").unwrap()
    );

    let corpus: Value =
        serde_json::from_slice(&fs::read(fixture).unwrap()).unwrap();

    for case in corpus.as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let query = case["query"].as_str().unwrap();
        let prompt = case["prompt"].as_str().unwrap();
        let query_words = coverage_words_v176(query);

        let clauses = prompt.replace(", ", "\n")
            .replace("; ", "\n")
            .replace(" and ", "\n")
            .replace(". ", "\n");

        let mut facets: Vec<HashSet<String>> = clauses.lines()
            .map(coverage_words_v176)
            .filter(|f| !f.is_empty() && !f.is_disjoint(&query_words))
            .collect();

        if facets.is_empty() { facets.push(query_words.clone()); }

        let mut blocks = Vec::new();

        for (source,row) in case["sources"].as_array()
            .unwrap().iter().enumerate()
        {
            let raw = row["raw"].as_str().unwrap();
            assert_eq!(
                sha256_text_v1(raw),
                row["raw_sha256"].as_str().unwrap()
            );

            let text = extract_web_text_v176(raw,row["mime"].as_str());

            for block in coverage_blocks_v176(&text) {
                blocks.push((
                    source,
                    block.to_owned(),
                    coverage_words_v176(block),
                    block.chars().count() + 20
                ));
            }
        }

        let vocabulary: HashSet<_> = facets.iter()
            .flat_map(|f| f.iter().cloned()).collect();

        let weights: std::collections::HashMap<_,_> = vocabulary.iter()
            .map(|w| {
                let df = blocks.iter()
                    .filter(|b| b.2.contains(w)).count();
                (w.clone(),1 + (blocks.len()/(df+1)).min(12))
            }).collect();

        let scores: Vec<Vec<usize>> = facets.iter().map(|facet| {
            blocks.iter().map(|b| {
                let hits = facet.intersection(&b.2).count();
                let weight: usize = facet.intersection(&b.2)
                    .map(|w| weights[w]).sum();
                weight + 4 * hits * hits
            }).collect()
        }).collect();

        for budget in [3605_usize,14824] {
            let started = Instant::now();
            let mut picked = vec![false; blocks.len()];
            let mut spent = 0;

            // First give each requested topic an evidence allocation.
            for scores in &scores {
                let best = (0..blocks.len())
                    .filter(|&i| scores[i] > 0 &&
                        (picked[i] || blocks[i].3 <= budget-spent))
                    .max_by_key(|&i| (
                        scores[i],std::cmp::Reverse(i)
                    ));

                if let Some(i) = best {
                    if !picked[i] {
                        picked[i] = true;
                        spent += blocks[i].3;
                    }
                }
            }

            // Then prioritize additional coverage over repetition.
            loop {
                let mut covered = HashSet::new();
                let mut counts = vec![0_usize; facets.len()];

                for (i,b) in blocks.iter().enumerate() {
                    if picked[i] {
                        covered.extend(b.2.iter().cloned());
                        for (f,score) in scores.iter().enumerate() {
                            if score[i] > 0 { counts[f] += 1; }
                        }
                    }
                }

                let best = (0..blocks.len())
                    .filter(|&i| !picked[i] && blocks[i].3 <= budget-spent)
                    .map(|i| {
                        let novelty: usize = blocks[i].2.iter()
                            .filter(|w| !covered.contains(*w))
                            .filter_map(|w| weights.get(w)).sum();

                        let relevance = scores.iter().enumerate()
                            .map(|(f,score)| score[i]*100/(1+counts[f]))
                            .max().unwrap_or(0);

                        (i,novelty*100 + relevance)
                    })
                    .filter(|&(_,score)| score > 0)
                    .max_by_key(|&(i,score)| (
                        score,std::cmp::Reverse(i)
                    ));

                match best {
                    Some((i,_)) => {
                        picked[i] = true;
                        spent += blocks[i].3;
                    }
                    None => break
                }
            }

            let evidence = blocks.iter().enumerate()
                .filter(|(i,_)| picked[*i])
                .map(|(_,b)| b.1.as_str())
                .collect::<Vec<_>>()
                .join("\n[... omitted ...]\n");

            assert!(evidence.chars().count() <= budget);

            println!(
                "\nCASE={id} BUDGET={budget} EVIDENCE_CHARS={} \
                 FACETS={} SELECTED_BLOCKS={} SELECTION_MS={}",
                evidence.chars().count(),facets.len(),
                picked.iter().filter(|&&x| x).count(),
                started.elapsed().as_millis()
            );

            if id == "technical_lookup" {
                let text = clean_v1(&evidence);

                // Diagnostic checks only; never used in selection.
                for (label,pattern) in [
                    ("FOLD",r"offset from before.{0,180}fold\s*=\s*1"),
                    ("ENVIRONMENT",r"environment variable\s+PYTHONTZPATH"),
                    ("FALLBACK",r"falls? back|fall back"),
                    ("MISSING_DATA",r"raise\s+(?:a\s+)?ZoneInfoNotFoundError"),
                    ("CACHE_API",r"clear_cache\s*\("),
                    ("CACHE_KEYS",r"only_keys"),
                    ("PATH_CACHE_WARNING",r"will not invalidate"),
                    ("PATH_ARGUMENT",r"sequence of strings|not a string"),
                ] {
                    println!("{label}={}",
                        if Regex::new(pattern).unwrap().is_match(&text) {
                            "FOUND"
                        } else { "NOT_FOUND" }
                    );
                }

                if budget == 3605 {
                    println!("COMPACT_EVIDENCE:\n{evidence}");
                }
            }
        }
    }
}
