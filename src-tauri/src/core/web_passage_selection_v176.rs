use std::collections::{HashMap, HashSet};
use std::cmp::Reverse;

const OMISSION: &str = "\n[... omitted ...]\n";

fn terms(text: &str, limit: usize) -> HashSet<String> {
    let mut result = HashSet::new();

    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        let word = raw.to_lowercase();
        let n = word.chars().count();

        if n == 0 || n > 64 ||
            (n == 1 && !word.chars().all(|c| c.is_numeric())) ||
            matches!(word.as_str(),
                "the" | "and" | "for" | "with" | "from" |
                "this" | "that" | "are" | "was" | "were" |
                "what" | "which" | "how" | "about" | "to" |
                "of" | "in" | "on" | "at" | "is" | "or" | "an"
            )
        {
            continue;
        }

        result.insert(word);
        if result.len() >= limit { break; }
    }

    result
}

// Character-safe, ordered spans over the original cleaned text.
fn passages(text: &str) -> Vec<(usize, usize, usize)> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut count = 0;

    for (i,c) in text.char_indices() {
        count += 1;
        if (count >= 850 && c.is_whitespace()) || count >= 1200 {
            let end = i + c.len_utf8();
            result.push((start,end,count));
            start = end;
            count = 0;
        }
    }

    if start < text.len() {
        result.push((start,text.len(),count));
    }

    result
}

fn reserve(
    selected: &mut [bool],
    spans: &[(usize,usize,usize)],
    i: usize,
    spent: &mut usize,
    limit: usize,
) {
    if selected[i] { return; }

    let cost = spans[i].2 + OMISSION.chars().count();

    if cost <= limit.saturating_sub(*spent) {
        selected[i] = true;
        *spent += cost;
    }
}

pub(crate) fn select_passages(
    text: &str,
    query: &str,
    highlight: &str,
    budget: usize,
) -> String {
    if text.chars().count() <= budget {
        return text.to_owned();
    }

    let fallback = || text.chars().take(budget).collect::<String>();
    let query_terms = terms(query, 96);
    let hint_terms = terms(highlight, 96);

    if query_terms.is_empty() {
        return fallback();
    }

    let spans = passages(text);
    let words: Vec<_> = spans.iter()
        .map(|&(a,b,_)| terms(&text[a..b], usize::MAX))
        .collect();

    let frequency: HashMap<String,usize> = query_terms.iter()
        .map(|term| {
            let count = words.iter().filter(|w| w.contains(term)).count();
            (term.clone(), count)
        }).collect();

    let mut ranked = Vec::new();

    for (i,w) in words.iter().enumerate() {
        let hits = w.intersection(&query_terms).count();
        if hits == 0 { continue; }

        let score: usize = w.intersection(&query_terms)
            .map(|term| {
                1 + (words.len() / (frequency[term] + 1)).min(8)
            }).sum();

        let hints = w.intersection(&hint_terms).count();
        ranked.push((i,score,hits,hints));
    }

    if ranked.is_empty() {
        return fallback();
    }

    // Query relevance is authoritative; highlights break ties only.
    ranked.sort_by_key(|&(i,score,hits,hints)| {
        (Reverse(score),Reverse(hits),Reverse(hints),i)
    });

    let mut selected = vec![false; spans.len()];
    let mut spent = 0;

    // Reserve some capacity for surrounding factual context.
    for &(i,_,_,_) in &ranked {
        reserve(&mut selected,&spans,i,&mut spent,budget * 3 / 4);
    }

    if !selected.iter().any(|v| *v) {
        reserve(&mut selected,&spans,ranked[0].0,&mut spent,budget);
    }

    for &(i,_,_,_) in &ranked {
        if selected[i] {
            if i > 0 {
                reserve(&mut selected,&spans,i-1,&mut spent,budget);
            }
            if i+1 < spans.len() {
                reserve(&mut selected,&spans,i+1,&mut spent,budget);
            }
        }
    }

    for &(i,_,_,_) in &ranked {
        reserve(&mut selected,&spans,i,&mut spent,budget);
    }

    // Merge adjacent spans and preserve original document order.
    let mut parts = Vec::new();
    let mut start = None;

    for (i,&chosen) in selected.iter().enumerate() {
        if chosen {
            if start.is_none() { start = Some(spans[i].0); }
        } else if let Some(a) = start.take() {
            parts.push(text[a..spans[i-1].1].trim());
        }
    }

    if let Some(a) = start {
        parts.push(text[a..].trim());
    }

    if parts.is_empty() { return fallback(); }

    let output = parts.join(OMISSION);
    debug_assert!(output.chars().count() <= budget);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_late_and_separated_facts() {
        let noise = "Generic unrelated navigation information. ".repeat(600);

        let raw = format!(
            "{} Stockholm jazz concert November 24 costs 250 SEK. \
             {} Stockholm comedy November 28 costs 300 SEK. {}",
            noise,noise,noise
        );

        let output = select_passages(
            &raw,"Stockholm jazz comedy November",
            "FABRICATED_BROKER_ONLY_PRICE",6000
        );

        assert!(output.contains("250 SEK"));
        assert!(output.contains("300 SEK"));
        assert!(!output.contains("FABRICATED_BROKER_ONLY_PRICE"));
        assert!(output.chars().count() <= 6000);

        for excerpt in output.split(OMISSION) {
            assert!(raw.contains(excerpt));
        }

        assert_eq!(output,select_passages(
            &raw,"Stockholm jazz comedy November",
            "FABRICATED_BROKER_ONLY_PRICE",6000
        ));
    }

    #[test]
    fn highlight_cannot_override_query_relevance() {
        let noise = "Unrelated generic material. ".repeat(800);
        let raw = format!(
            "Misleading discount claim. {} Göteborg museum admission 120 SEK. {}",
            noise,noise
        );

        let output = select_passages(
            &raw,"Göteborg museum admission",
            "Misleading discount claim",4000
        );

        assert!(output.contains("120 SEK"));
        assert!(!output.contains("Misleading discount claim"));
    }

    #[test]
    fn no_match_preserves_legacy_fallback() {
        let raw = "Regular document content. ".repeat(1000);
        assert_eq!(
            select_passages(&raw,"unfindable","document",4000),
            raw.chars().take(4000).collect::<String>()
        );
    }

    #[test]
    fn preserves_short_documents_and_unicode_boundaries() {
        assert_eq!(
            select_passages("Café: 250 SEK","café","",12000),
            "Café: 250 SEK"
        );

        let raw = "åäö".repeat(10000);
        let output = select_passages(&raw,"missing","",1200);
        assert_eq!(output.chars().count(),1200);
    }
}
