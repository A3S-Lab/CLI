pub(super) const REPORT_CSS: &str = r#"
:root {
  --ink: #11233f;
  --muted: #657186;
  --paper: #f4f0e8;
  --surface: #fffdf8;
  --line: #d9d4c8;
  --accent: #16728f;
  --accent-soft: #dceef1;
  --signal: #e66d42;
  --navy: #0b1e37;
  --warning: #8a4b12;
  --warning-bg: #fff3df;
  --shadow: 0 22px 64px rgba(11, 30, 55, .1);
  color-scheme: light;
}
body.report-qualified { --accent: #8d5e12; --accent-soft: #f8edcf; --signal: #e5a52a; --navy: #302713; }
body.report-degraded { --accent: #9a5719; --accent-soft: #f8e9d5; --signal: #eea33b; --navy: #382719; --warning-bg: #fff0d7; }
* { box-sizing: border-box; }
html { scroll-behavior: smooth; }
body{margin:0;background:var(--paper);color:var(--ink);font-family:Inter,ui-sans-serif,-apple-system,BlinkMacSystemFont,'Segoe UI','PingFang SC','Noto Sans CJK SC',sans-serif;line-height:1.72;text-rendering:optimizeLegibility}
a { color: var(--accent); text-decoration-thickness: .08em; text-underline-offset: .18em; overflow-wrap: anywhere; }
a:hover { filter: brightness(.78); }
a:focus-visible { outline: 3px solid #f4a261; outline-offset: 3px; border-radius: 3px; }
.hero{background:var(--navy);color:#fff;overflow:hidden;position:relative}
.hero::after { content: ''; position: absolute; width: 520px; height: 520px; right: -180px; top: -210px; border: 1px solid rgba(255,255,255,.1); border-radius: 50%; box-shadow: 0 0 0 74px rgba(255,255,255,.025), 0 0 0 148px rgba(255,255,255,.018); }
.hero-inner { position: relative; z-index: 1; max-width: 1200px; margin: 0 auto; padding: 72px 32px 58px; }
.hero-grid { display: grid; grid-template-columns: minmax(0, 1.55fr) minmax(280px, .65fr); gap: 64px; align-items: end; }
.eyebrow { display: flex; align-items: center; gap: 11px; margin: 0 0 24px; color: #9adce9; font-size: .74rem; font-weight: 800; letter-spacing: .17em; text-transform: uppercase; }
.eyebrow::before { content: ''; width: 38px; height: 3px; border-radius: 99px; background: var(--signal); }
.hero h1 { max-width: 900px; margin: 0; font-family: Georgia,'Noto Serif CJK SC',serif; font-size: clamp(2.5rem, 5.5vw, 5.6rem); font-weight: 500; line-height: 1.01; letter-spacing: -.045em; text-wrap: balance; overflow-wrap: anywhere; }
.hero-thesis { max-width: 68ch; margin: 25px 0 0; color: #c8d7e5; font-size: clamp(.98rem, 1.5vw, 1.14rem); }
.evidence-profile { border-top: 1px solid rgba(255,255,255,.24); padding-top: 20px; }
.profile-label { margin: 0 0 18px; color: #91adbf; font-size: .68rem; font-weight: 800; letter-spacing: .15em; text-transform: uppercase; }
.profile-grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 16px; }
.profile-grid div { min-width: 0; }
.profile-grid strong { display: block; color: #fff; font-family: Georgia,serif; font-size: clamp(1.65rem, 3vw, 2.6rem); font-weight: 500; line-height: 1; font-variant-numeric: tabular-nums; }
.profile-grid span { display: block; margin-top: 8px; color: #a9bdcd; font-size: .68rem; line-height: 1.35; text-transform: uppercase; letter-spacing: .06em; }
.signal-row { display: flex; flex-wrap: wrap; gap: 9px; margin-top: 30px; }
.signal { display: inline-flex; align-items: center; gap: 7px; padding: 7px 11px; border: 1px solid rgba(255,255,255,.18); border-radius: 999px; background: rgba(255,255,255,.06); color: #d7e5ed; font-size: .76rem; }
.signal b { color: #fff; }
main { max-width: 1200px; margin: 0 auto; padding: 48px 32px 88px; }
.report-shell { display: grid; grid-template-columns: 230px minmax(0, 1fr); gap: 48px; align-items: start; min-width: 0; }
.rail { position: sticky; top: 20px; min-width: 0; padding: 9px 0 28px; }
.rail-label { margin: 0 0 15px; color: var(--accent); font-size: .68rem; font-weight: 850; letter-spacing: .15em; text-transform: uppercase; }
.toc { display: grid; gap: 2px; }
.toc a { display: grid; grid-template-columns: 28px minmax(0, 1fr); gap: 8px; align-items: baseline; padding: 8px 7px; border-left: 2px solid transparent; color: #415169; font-size: .79rem; line-height: 1.3; text-decoration: none; }
.toc a:hover { border-left-color: var(--accent); background: rgba(255,255,255,.48); color: var(--ink); filter: none; }
.toc a span { color: #98a0aa; font-size: .62rem; font-variant-numeric: tabular-nums; }
.rail-stat { display: flex; flex-direction: column; gap: 2px; margin: 24px 7px 0; padding-top: 18px; border-top: 1px solid #cbc5b8; }
.rail-stat dt { order: 2; color: var(--muted); font-size: .65rem; font-weight: 800; letter-spacing: .08em; text-transform: uppercase; }
.rail-stat dd { order: 1; margin: 0; color: var(--navy); font-family: Georgia,serif; font-size: 2rem; line-height: 1; font-variant-numeric: tabular-nums; }
article{min-width:0;max-width:100%;overflow-wrap:anywhere;word-break:break-word}
.report-section { position: relative; scroll-margin-top: 24px; margin: 0 0 52px; }
.section-index { margin-bottom: 12px; color: var(--signal); font-size: .67rem; font-weight: 900; letter-spacing: .14em; }
.report-section > h2 { max-width: 20ch; margin: 0 0 24px; font-family: Georgia,'Noto Serif CJK SC',serif; font-size: clamp(1.8rem, 3.5vw, 2.85rem); font-weight: 500; line-height: 1.12; letter-spacing: -.025em; text-wrap: balance; }
.section-body > p, .prose > p { max-width: 74ch; }
p, li { font-size: 1rem; }
li + li { margin-top: .48em; }
ul, ol { padding-left: 1.3rem; }
strong { color: #071a30; }
blockquote { margin: 28px 0; padding: 18px 22px; border-left: 4px solid var(--accent); background: var(--accent-soft); color: #29465a; }
code { padding: .12em .36em; border-radius: 5px; background: #edf0f2; color: #18354d; font-size: .9em; }
pre { max-width: 100%; padding: 18px; border-radius: 12px; background: #0b1f38; color: #ecf4f6; white-space: pre-wrap; word-break: break-word; overflow: auto; }
.section--lead { padding: 28px 32px; border-left: 5px solid var(--signal); background: rgba(255,255,255,.55); }
.section--lead p:first-child { margin-top: 0; font-family: Georgia,'Noto Serif CJK SC',serif; font-size: clamp(1.22rem,2.2vw,1.55rem); line-height: 1.55; color: #31445b; }
.section--summary { padding: clamp(28px, 4.5vw, 48px); border: 1px solid rgba(210,204,191,.9); border-radius: 18px; background: var(--surface); box-shadow: var(--shadow); }
.section--summary .section-index { margin-top: -4px; }
.section--summary .section-body > p:first-child { max-width: 62ch; color: var(--muted); }
.section--summary .section-body > ul { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 0 30px; margin: 28px 0 0; padding: 0; list-style: none; counter-reset: summary; }
.section--summary .section-body > ul > li { position: relative; margin: 0; padding: 18px 0 18px 38px; border-top: 1px solid var(--line); counter-increment: summary; }
.section--summary .section-body > ul > li::before { content: counter(summary, decimal-leading-zero); position: absolute; left: 0; top: 20px; color: var(--accent); font-size: .68rem; font-weight: 900; font-variant-numeric: tabular-nums; }
.section--findings { padding: 10px 0 0; }
.findings-lead { margin-bottom: 24px; }
.findings-list { border-top: 1px solid var(--line); }
.finding { display: grid; grid-template-columns: 68px minmax(0, 1fr); gap: 22px; padding: 34px 0 38px; border-bottom: 1px solid var(--line); }
.finding-number { color: var(--accent); font-family: Georgia,serif; font-size: 1.55rem; font-variant-numeric: tabular-nums; }
.finding-content h3 { max-width: 28ch; margin: 0 0 14px; font-size: clamp(1.16rem,2vw,1.45rem); line-height: 1.25; }
.finding-content > p:first-of-type { max-width: 72ch; margin-top: 0; font-family: Georgia,'Noto Serif CJK SC',serif; font-size: 1.08rem; color: #2f4359; }
.finding-content ul { columns: 2; column-gap: 34px; }
.finding-content li { break-inside: avoid; margin-bottom: .55em; color: #425269; font-size: .91rem; }
.section--matrix { padding: clamp(24px, 4vw, 42px); border-radius: 16px; background: #e9eef0; }
.section--matrix > h2 { max-width: none; }
.table-wrap { width: 100%; overflow-x: auto; border: 1px solid #cbd3d6; border-radius: 12px; background: #fff; box-shadow: 0 8px 30px rgba(11,30,55,.06); }
.table-wrap:focus-visible { outline: 3px solid var(--accent); outline-offset: 3px; }
table { width: 100%; min-width: 720px; border-spacing: 0; border-collapse: separate; }
th, td { min-width: 130px; padding: 13px 14px; border-right: 1px solid #d9dfe1; border-bottom: 1px solid #d9dfe1; text-align: left; vertical-align: top; font-size: .83rem; overflow-wrap: anywhere; }
th { background: #dbe5e8; color: #18354d; font-size: .69rem; letter-spacing: .055em; text-transform: uppercase; }
tr:last-child td { border-bottom: 0; }
th:last-child, td:last-child { border-right: 0; }
tr:nth-child(even) td { background: #fafbfa; }
.section--caveats { padding: clamp(28px, 4vw, 44px); border-top: 5px solid #d28b32; background: var(--warning-bg); color: #5f3a18; }
.section--caveats > h2 { color: #5a3212; }
.section--caveats li { max-width: 76ch; }
.section--confidence { padding: clamp(30px, 5vw, 50px); border-radius: 16px; background: var(--navy); color: #d9e4ec; }
.section--confidence .section-index { color: #82d0df; }
.section--confidence > h2, .section--confidence strong { color: #fff; }
.section--confidence p { max-width: 67ch; font-family: Georgia,'Noto Serif CJK SC',serif; font-size: 1.08rem; }
.section--sources .section-body > ul { display: grid; grid-template-columns: repeat(2, minmax(0,1fr)); gap: 12px; padding: 0; list-style: none; counter-reset: source; }
.section--sources .section-body > ul > li { position: relative; margin: 0; min-width: 0; padding: 18px 18px 18px 48px; border-top: 1px solid var(--line); background: rgba(255,255,255,.46); counter-increment: source; color: #47566a; font-size: .82rem; }
.section--sources .section-body > ul > li::before { content: counter(source, decimal-leading-zero); position: absolute; left: 16px; top: 19px; color: var(--signal); font-size: .68rem; font-weight: 900; }
.section--sources a { font-weight: 750; }
.section--narrative { padding: 8px 0; }
.footer-note { max-width: 1200px; margin: 0 auto; padding: 0 32px 34px; color: #6c746f; font-size: .74rem; }
@media(max-width:900px) {
  .hero-grid { grid-template-columns: 1fr; gap: 38px; }
  .evidence-profile { max-width: 620px; }
  .report-shell { grid-template-columns: 1fr; gap: 28px; }
  .rail { position: static; padding: 0; }
  .rail-label { margin-left: 7px; }
  .toc { display: flex; gap: 4px; padding: 0 0 10px; overflow-x: auto; }
  .toc a { flex: 0 0 auto; max-width: 220px; border-left: 0; border-bottom: 2px solid transparent; background: rgba(255,255,255,.42); }
  .toc a:hover { border-left-color: transparent; border-bottom-color: var(--accent); }
  .rail-stat{flex-direction:row;align-items:baseline;gap:8px;margin:8px 7px 0;padding:12px 0 0}
  .rail-stat dt { order: 1; }
  .rail-stat dd { order: 2; font-size: 1.35rem; }
}
@media(max-width:820px) {
  .hero-inner { padding: 50px 20px 42px; }
  main { padding: 34px 14px 60px; }
  .section--summary .section-body > ul { grid-template-columns: 1fr; }
  .finding-content ul { columns: 1; }
  .section--sources .section-body > ul { grid-template-columns: 1fr; }
  .table-wrap::before { content: '← 横向滑动查看全部列 / swipe to inspect →'; position: sticky; left: 0; z-index: 1; display: block; width: min(100%, 720px); padding: 8px 12px; border-bottom: 1px solid #d9dfe1; background: #f7faf9; color: var(--muted); font-size: .68rem; font-weight: 750; letter-spacing: .025em; }
}
@media(max-width:480px) {
  .hero h1 { font-size: clamp(2.1rem, 11vw, 3.15rem); }
  .hero-thesis { font-size: .96rem; }
  .profile-grid { gap: 8px; }
  .profile-grid strong { font-size: 1.65rem; }
  .profile-grid span { font-size: .58rem; }
  .signal-row { gap: 7px; }
  .signal { font-size: .69rem; }
  .report-section { margin-bottom: 38px; }
  .section--summary, .section--matrix, .section--caveats, .section--confidence { padding: 24px 18px; border-radius: 12px; }
  .finding { grid-template-columns: 42px minmax(0,1fr); gap: 10px; padding: 28px 0 31px; }
  .finding-number { font-size: 1.15rem; }
  p, li { font-size: .96rem; }
  th, td { padding: 10px; font-size: .78rem; }
}
@media(prefers-reduced-motion:reduce) { html { scroll-behavior: auto; } }
@media print {
  body { background: #fff; color: #000; }
  .hero { background: #fff; color: #000; border-bottom: 2px solid #000; }
  .hero::after, .toc { display: none; }
  .hero-inner { max-width: none; padding: 24px 0; }
  .eyebrow, .hero-thesis, .profile-label, .profile-grid span, .signal { color: #000; }
  .profile-grid strong, .signal b { color: #000; }
  .signal { border-color: #777; }
  main { max-width: none; padding: 20px 0; }
  .report-shell { display: block; }
  .rail { display: none; }
  .report-section { break-inside: auto; margin-bottom: 28px; box-shadow: none; }
  .section--summary, .section--matrix, .section--caveats, .section--confidence { padding: 16px 0; border: 0; background: #fff; color: #000; }
  .section--confidence > h2, .section--confidence strong { color: #000; }
  .finding, tr { break-inside: avoid; }
  h2, h3 { break-after: avoid; }
  a { color: #000; text-decoration: underline; }
  .table-wrap { overflow: visible; border: 1px solid #777; box-shadow: none; }
  table { min-width: 0; }
}
"#;
