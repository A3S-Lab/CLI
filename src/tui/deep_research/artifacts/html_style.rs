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

/* report-master semantic art direction. These classes are emitted only from
   schema-validated enums; model-authored CSS and arbitrary class names never
   enter the document. */
body.palette-graphite { --ink:#171b23; --muted:#626a76; --paper:#eef0f2; --surface:#fff; --line:#cfd4da; --accent:#3568a8; --accent-soft:#e1eaf5; --signal:#dc5d45; --navy:#151a22; }
body.palette-forest { --ink:#18302a; --muted:#64756e; --paper:#eef2e9; --surface:#fbfcf7; --line:#ced8cb; --accent:#23705a; --accent-soft:#dcece4; --signal:#c66b3d; --navy:#153a32; }
body.palette-amber { --ink:#342517; --muted:#786b5d; --paper:#f4eee2; --surface:#fffaf1; --line:#ded0bb; --accent:#a44d22; --accent-soft:#f5dfc5; --signal:#cf7b16; --navy:#3a2618; }
body.palette-plum { --ink:#30223a; --muted:#75687d; --paper:#f2edf3; --surface:#fffaff; --line:#d9cddd; --accent:#80528e; --accent-soft:#eaddec; --signal:#c85f75; --navy:#32203c; }

body.hero-statement .hero-inner { padding-top: 92px; padding-bottom: 70px; }
body.hero-statement .hero-grid { display:block; text-align:center; }
body.hero-statement .hero h1, body.hero-statement .hero-thesis { margin-left:auto; margin-right:auto; }
body.hero-statement .eyebrow, body.hero-statement .signal-row { justify-content:center; }
body.hero-statement .evidence-profile { max-width:720px; margin:46px auto 0; }
body.hero-metrics .hero-grid { grid-template-columns:minmax(260px,.58fr) minmax(0,1.42fr); }
body.hero-metrics .hero-grid > div { order:2; }
body.hero-metrics .evidence-profile { order:1; padding:26px; border:1px solid rgba(255,255,255,.2); background:rgba(255,255,255,.055); }
body.hero-metrics .profile-grid { grid-template-columns:1fr; gap:22px; }
body.hero-metrics .profile-grid div { display:grid; grid-template-columns:70px minmax(0,1fr); align-items:baseline; gap:12px; padding-bottom:17px; border-bottom:1px solid rgba(255,255,255,.14); }
body.hero-metrics .profile-grid div:last-child { padding-bottom:0; border-bottom:0; }
body.hero-metrics .profile-grid span { margin:0; }

body.density-compact .hero-inner { padding-top:48px; padding-bottom:42px; }
body.density-compact main { padding-top:34px; padding-bottom:58px; }
body.density-compact .report-section { margin-bottom:34px; }
body.density-compact p, body.density-compact li { font-size:.94rem; }
body.density-compact .finding { padding:24px 0 27px; }
body.density-spacious .hero-inner { padding-top:94px; padding-bottom:76px; }
body.density-spacious main { padding-top:66px; padding-bottom:118px; }
body.density-spacious .report-shell { gap:68px; }
body.density-spacious .report-section { margin-bottom:76px; }
body.density-spacious p, body.density-spacious li { font-size:1.04rem; line-height:1.8; }

body.archetype-analytical { background:#eef1f4; font-family:Inter,ui-sans-serif,-apple-system,BlinkMacSystemFont,'Segoe UI','PingFang SC',sans-serif; }
body.archetype-analytical .hero::after { width:438px; height:438px; right:-120px; top:-180px; border-radius:0; transform:rotate(24deg); box-shadow:0 0 0 54px rgba(255,255,255,.025),0 0 0 108px rgba(255,255,255,.015); }
body.archetype-analytical .hero h1, body.archetype-analytical .report-section > h2, body.archetype-analytical .rail-stat dd, body.archetype-analytical .finding-number { font-family:Inter,ui-sans-serif,sans-serif; font-weight:760; letter-spacing:-.04em; }
body.archetype-analytical .report-shell { grid-template-columns:255px minmax(0,1fr); }
body.archetype-analytical .section--summary { border-radius:4px; border-top:6px solid var(--accent); box-shadow:0 16px 42px rgba(17,25,36,.08); }
body.archetype-analytical .section--matrix { border-radius:4px; background:#dfe5ea; }
body.archetype-analytical .finding { grid-template-columns:54px minmax(0,1fr); }
body.archetype-analytical .finding-number { font-size:.84rem; padding-top:4px; color:var(--signal); }
body.archetype-analytical .section--confidence { border-radius:4px; }

body.archetype-chronicle { --shadow:none; }
body.archetype-chronicle .hero-inner { max-width:980px; }
body.archetype-chronicle .hero h1 { font-size:clamp(2.7rem,6.3vw,6.4rem); font-style:italic; }
body.archetype-chronicle main { max-width:980px; }
body.archetype-chronicle .report-shell { display:block; }
body.archetype-chronicle .rail { position:static; display:grid; grid-template-columns:120px minmax(0,1fr) 90px; gap:18px; align-items:start; margin-bottom:54px; padding-bottom:22px; border-bottom:1px solid var(--line); }
body.archetype-chronicle .toc { display:flex; gap:5px; overflow-x:auto; }
body.archetype-chronicle .toc a { flex:0 0 auto; max-width:210px; border-left:0; border-bottom:2px solid transparent; }
body.archetype-chronicle .rail-stat { margin:0; padding:0; border:0; }
body.archetype-chronicle .report-section { margin-left:34px; padding:4px 0 58px 54px; border-left:1px solid var(--accent); }
body.archetype-chronicle .section-index { position:absolute; left:-18px; top:0; width:35px; height:35px; display:grid; place-items:center; margin:0; border:1px solid var(--accent); border-radius:50%; background:var(--paper); color:var(--accent); }
body.archetype-chronicle .section--summary, body.archetype-chronicle .section--matrix, body.archetype-chronicle .section--caveats, body.archetype-chronicle .section--confidence { border-radius:0; box-shadow:none; }

body.archetype-executive { --shadow:none; background:var(--surface); }
body.archetype-executive .hero { border-bottom:1px solid var(--line); background:var(--surface); color:var(--ink); }
body.archetype-executive .hero::after { display:none; }
body.archetype-executive .hero-inner { padding-top:52px; padding-bottom:46px; }
body.archetype-executive .hero h1 { max-width:760px; color:var(--ink); font-family:Inter,ui-sans-serif,sans-serif; font-size:clamp(2.35rem,4.7vw,4.7rem); font-weight:730; }
body.archetype-executive .eyebrow { color:var(--accent); }
body.archetype-executive .hero-thesis { color:var(--muted); }
body.archetype-executive .signal { border-color:var(--line); background:var(--paper); color:var(--muted); }
body.archetype-executive .signal b, body.archetype-executive .profile-grid strong { color:var(--ink); }
body.archetype-executive .profile-label, body.archetype-executive .profile-grid span { color:var(--muted); }
body.archetype-executive .evidence-profile { border-top-color:var(--line); }
body.archetype-executive main { max-width:1120px; }
body.archetype-executive .report-section > h2 { font-family:Inter,ui-sans-serif,sans-serif; font-weight:720; }
body.archetype-executive .section--summary { padding-left:0; padding-right:0; border:0; border-top:5px solid var(--accent); border-radius:0; }
body.archetype-executive .section--confidence { border-radius:0; }

body.archetype-field-notes { background-color:var(--paper); background-image:linear-gradient(rgba(80,95,86,.07) 1px,transparent 1px); background-size:100% 28px; }
body.archetype-field-notes .hero { border-bottom:2px dashed var(--accent); background:var(--paper); color:var(--ink); }
body.archetype-field-notes .hero::after { width:358px; height:358px; border:2px dashed var(--accent); box-shadow:none; opacity:.25; }
body.archetype-field-notes .hero h1 { color:var(--ink); font-style:italic; transform:rotate(-.35deg); }
body.archetype-field-notes .eyebrow { color:var(--accent); font-family:ui-monospace,SFMono-Regular,Menlo,monospace; }
body.archetype-field-notes .hero-thesis, body.archetype-field-notes .profile-label, body.archetype-field-notes .profile-grid span { color:var(--muted); }
body.archetype-field-notes .profile-grid strong, body.archetype-field-notes .signal b { color:var(--ink); }
body.archetype-field-notes .evidence-profile { border-top-color:var(--line); }
body.archetype-field-notes .signal { border-color:var(--line); background:rgba(255,255,255,.42); color:var(--muted); border-radius:3px; }
body.archetype-field-notes .report-section { padding:26px 30px; border:1px dashed #aeb9ae; background:rgba(255,255,255,.64); box-shadow:4px 5px 0 rgba(63,87,75,.07); }
body.archetype-field-notes .section--summary, body.archetype-field-notes .section--matrix, body.archetype-field-notes .section--caveats, body.archetype-field-notes .section--confidence { border-radius:2px; }
body.archetype-field-notes .section-index { font-family:ui-monospace,SFMono-Regular,Menlo,monospace; }

body.mode-narrative .report-section > h2 { max-width:26ch; }
body.mode-narrative .section-index::after { content:' —'; }
body.mode-instructional .section-index { display:inline-block; padding:4px 8px; border:1px solid var(--accent); border-radius:999px; }
body.mode-pyramid .section--summary { border-top:6px solid var(--signal); }
body.mode-briefing .report-section > h2 { max-width:25ch; }
body.stance-safe .hero::after { opacity:.45; }
body.stance-bold .hero h1 { max-width:980px; }
body.stance-bold .eyebrow::before { width:64px; }
body.stance-bold .section-index { color:var(--accent); }
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
  body.hero-metrics .hero-grid { grid-template-columns:1fr; }
  body.hero-metrics .hero-grid > div, body.hero-metrics .evidence-profile { order:initial; }
  body.hero-metrics .profile-grid { grid-template-columns:repeat(3,1fr); }
  body.hero-metrics .profile-grid div { display:block; padding:0; border:0; }
  body.archetype-chronicle .rail { display:block; }
  body.archetype-chronicle .rail-label { margin-left:0; }
  body.archetype-chronicle .rail-stat { margin-top:12px; }
}
@media(max-width:820px) {
  .hero-inner { padding: 50px 20px 42px; }
  main { padding: 34px 14px 60px; }
  .section--summary .section-body > ul { grid-template-columns: 1fr; }
  .finding-content ul { columns: 1; }
  .section--sources .section-body > ul { grid-template-columns: 1fr; }
  .table-wrap::before { content: '← 横向滑动查看全部列 / swipe to inspect →'; position: sticky; left: 0; z-index: 1; display: block; width: min(100%, 720px); padding: 8px 12px; border-bottom: 1px solid #d9dfe1; background: #f7faf9; color: var(--muted); font-size: .68rem; font-weight: 750; letter-spacing: .025em; }
  body.archetype-chronicle .report-section { margin-left:16px; padding-left:34px; }
  body.archetype-field-notes .report-section { padding:22px 18px; }
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
  body, body.archetype-field-notes, body.archetype-analytical, body.archetype-executive { background: #fff; background-image: none; color: #000; }
  .hero { background: #fff; color: #000; border-bottom: 2px solid #000; }
  body.archetype-field-notes .hero, body.archetype-executive .hero { background:#fff; color:#000; border-bottom:2px solid #000; }
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
