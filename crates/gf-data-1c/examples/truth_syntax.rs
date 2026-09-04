//! Замер качества поиска по справке на эталонном наборе тем.
//!
//! Приёмка слоя — ДОЛЯ попаданий, а не «инструмент ответил». Наличие ответа ничего не
//! доказывает: подмена выглядит так же уверенно, как попадание, и именно поэтому ниже
//! считаются отдельно подмена (выдана чужая страница), ложный отказ (страница есть,
//! а отказали) и выдумка (страницы нет, а ответили).
//!
//! Эталон подаётся через `GTDATA_TRUTH`, база — через `GTDATA_KB`:
//!
//! ```text
//! GTDATA_KB=C:/Skynet/kb/1c-platform-help.db \
//! GTDATA_TRUTH=C:/Skynet/agents-data/_shared/model-eval/syntax-truth.json \
//! cargo run --example truth_syntax
//! ```
use std::collections::BTreeMap;

use gf_data_1c::tools::syntax::open_kb_for_eval;
use gf_data_1c::tools::syntaxindex::{search_help, HelpIndex};

#[derive(serde::Deserialize)]
struct Spec {
    #[serde(default)]
    expect: Vec<String>,
    #[serde(default)]
    refuse: bool,
}

#[derive(serde::Deserialize)]
struct Truth {
    themes: BTreeMap<String, Spec>,
}

fn main() {
    let Ok(path) = std::env::var("GTDATA_TRUTH") else {
        println!("эталон не подан (GTDATA_TRUTH) — замер пропущен");
        return;
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(e) => {
            println!("эталон не прочитан: {e}");
            return;
        }
    };
    let truth: Truth = match serde_json::from_str(&raw) {
        Ok(t) => t,
        Err(e) => {
            println!("эталон не разобран: {e}");
            return;
        }
    };

    let db = match open_kb_for_eval() {
        Ok(db) => db,
        Err(e) => {
            println!("{e}");
            return;
        }
    };
    let ix = HelpIndex::build(&db);

    let (mut hit, mut right_refuse, mut swap, mut false_refuse, mut invention) = (0, 0, 0, 0, 0);
    for (theme, spec) in &truth.themes {
        let (best, _) = search_help(&db, &ix, theme);
        match (&best, spec.refuse) {
            (None, true) => right_refuse += 1,
            (Some(p), true) => {
                invention += 1;
                println!("выдумка: {theme:?} -> {} ({})", p.object, p.title);
            }
            (None, false) => {
                false_refuse += 1;
                println!("ложный отказ: {theme:?}");
            }
            (Some(p), false) => {
                if spec.expect.iter().any(|w| w == &p.object) {
                    hit += 1;
                } else {
                    swap += 1;
                    println!("подмена: {theme:?} -> {} ({})", p.object, p.title);
                }
            }
        }
    }
    let total = truth.themes.len();
    let верно = hit + right_refuse;
    let мимо = swap + false_refuse + invention;
    println!(
        "тем {total} | попадание {hit} | верный отказ {right_refuse} | подмена {swap} | \
         ложный отказ {false_refuse} | выдумка {invention}"
    );
    println!(
        "ВЕРНО {верно} из {total} ({}%) | МИМО {мимо} ({}%)",
        верно * 100 / total,
        мимо * 100 / total
    );
}
