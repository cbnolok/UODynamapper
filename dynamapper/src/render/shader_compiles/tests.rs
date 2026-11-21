use super::*;

const TICK: Duration = Duration::from_millis(10);

fn waiting(ids: &[usize]) -> HashSet<usize> {
    ids.iter().copied().collect()
}

fn sync(book: &mut CompileBook, ids: &[usize], failed: &[usize], now: Instant) -> Vec<Finished> {
    book.sync_pipelines(&waiting(ids), |id| format!("pipeline {id}"), |id| failed.contains(&id), now)
}

#[test]
fn quick_compiles_never_show() {
    let start = Instant::now();
    let mut book = CompileBook::default();
    sync(&mut book, &[0, 1], &[], start);
    assert_eq!(book.view(start + TICK), None);
    sync(&mut book, &[], &[], start + TICK);
    // Within the linger, but no compile ran long enough to show the batch.
    assert_eq!(book.view(start + SHOW_AFTER + TICK), None);
}

#[test]
fn counts_done_and_total_across_the_batch() {
    let start = Instant::now();
    let mut book = CompileBook::default();
    sync(&mut book, &[0, 1, 2], &[], start);
    let later = start + SHOW_AFTER;
    let finished = sync(&mut book, &[1, 2, 3], &[], later);
    assert_eq!(finished, vec![Finished { name: "pipeline 0".into(), elapsed: SHOW_AFTER, failed: false }]);
    let view = book.view(later).unwrap();
    assert_eq!((view.done, view.total, view.failed), (1, 4, 0));
    // Oldest first.
    assert_eq!(view.listed.iter().map(|listed| listed.name.as_str()).collect::<Vec<_>>(),
        ["pipeline 1", "pipeline 2", "pipeline 3"]);
    assert_eq!(view.more, 0);
}

#[test]
fn failures_count_as_done_and_failed() {
    let start = Instant::now();
    let mut book = CompileBook::default();
    sync(&mut book, &[0, 1], &[], start);
    let finished = sync(&mut book, &[1], &[0], start + SHOW_AFTER);
    assert!(finished[0].failed);
    let view = book.view(start + SHOW_AFTER).unwrap();
    assert_eq!((view.done, view.total, view.failed), (1, 2, 1));
}

#[test]
fn lists_at_most_the_limit_and_counts_the_rest() {
    let start = Instant::now();
    let mut book = CompileBook::default();
    let ids = (0..MAX_LISTED + 2).collect::<Vec<_>>();
    sync(&mut book, &ids, &[], start);
    let view = book.view(start + SHOW_AFTER).unwrap();
    assert_eq!(view.listed.len(), MAX_LISTED);
    assert_eq!(view.more, 2);
}

#[test]
fn batch_lingers_then_closes() {
    let start = Instant::now();
    let mut book = CompileBook::default();
    sync(&mut book, &[0], &[], start);
    let ended = start + SHOW_AFTER;
    sync(&mut book, &[], &[], ended);
    // Shown, then kept through the linger: a compile queued in it joins the count.
    assert_eq!(book.view(ended).map(|view| (view.done, view.total)), Some((1, 1)));
    sync(&mut book, &[5], &[], ended + TICK);
    assert_eq!(book.view(ended + TICK).map(|view| (view.done, view.total)), Some((1, 2)));
    sync(&mut book, &[], &[], ended + 2 * TICK);
    assert!(book.view(ended + 2 * TICK + BATCH_LINGER - TICK).is_some());
    assert_eq!(book.view(ended + 2 * TICK + BATCH_LINGER), None);
    // The next compile starts a batch of its own.
    sync(&mut book, &[6], &[], ended + 3 * BATCH_LINGER);
    let view = book.view(ended + 3 * BATCH_LINGER + SHOW_AFTER).unwrap();
    assert_eq!((view.done, view.total), (0, 1));
}

#[test]
fn own_compiles_join_the_batch_and_end_once() {
    let start = Instant::now();
    let mut book = CompileBook::default();
    let id = book.begin_own("RIFE in-betweens: flow network".into(), start);
    sync(&mut book, &[0], &[], start);
    // A pipeline sync does not end an own compile.
    sync(&mut book, &[], &[], start + TICK);
    let view = book.view(start + SLOW_COMPILE).unwrap();
    assert_eq!(view.listed, vec![Listed { name: "RIFE in-betweens: flow network".into(), elapsed: SLOW_COMPILE }]);
    assert_eq!((view.done, view.total), (1, 2));
    let finished = book.end(CompileKey::Own(id), false, start + SLOW_COMPILE).unwrap();
    assert_eq!(finished.elapsed, SLOW_COMPILE);
    assert_eq!(book.end(CompileKey::Own(id), false, start + SLOW_COMPILE), None);
}

#[test]
fn guard_ends_its_compile_on_drop() {
    let name = "shader_compiles test guard";
    let listed = || book().lock().compiling.iter().any(|compile| compile.name == name);
    let guard = begin_shader_compile(name);
    assert!(listed());
    drop(guard);
    assert!(!listed());
    let id = shader_compile_started(name);
    assert!(listed());
    shader_compile_ended(id);
    shader_compile_ended(id);
    assert!(!listed());
}
