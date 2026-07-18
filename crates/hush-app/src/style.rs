//! All styling: vendored font data and the app-wide stylesheet, injected as
//! `<style>` blocks by the root component.

/// Vendored @font-face blocks with the woff2 bytes inlined as base64 data: URIs
/// at build time (build.rs) — no network needed to render the UI.
pub(crate) const FONT_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/fonts.css"));

pub(crate) const CSS: &str = r#"
:root{
  --bg:#070809; --card:#0e1217; --card2:#11161c; --line:#1b2128;
  --txt:#e9eef2; --mut:#5d6975; --acc:#37f2a6; --acc2:#0f7a52; --warn:#ff6b6b; --warn2:#6e2626;
}
*{ box-sizing:border-box; }
body{ margin:0; }
.root{
  font-family:'Chakra Petch', system-ui, sans-serif; color:var(--txt);
  height:100vh; padding:0; position:relative; overflow:hidden;
  display:flex; flex-direction:column;
  background:
    radial-gradient(130% 70% at 50% -8%, rgba(55,242,166,.12), transparent 58%),
    radial-gradient(90% 60% at 88% 112%, rgba(55,242,166,.05), transparent 60%),
    var(--bg);
}
.titlebar{ height:36px; display:flex; align-items:center; justify-content:space-between;
  padding:0 8px 0 18px; -webkit-user-select:none; user-select:none; position:relative; z-index:5; }
.tbrand{ font-family:'Chakra Petch'; font-weight:700; font-size:12px; letter-spacing:.30em; color:#8b97a2; }
.tbrand .dot{ color:var(--acc); text-shadow:0 0 8px var(--acc); }
.wctl{ display:flex; gap:2px; }
.wc{ width:30px; height:24px; border:none; background:transparent; color:#65717c; cursor:pointer;
  border-radius:6px; font-size:13px; line-height:1; transition:.12s; }
.wc:hover{ background:#1b2128; color:#dde4ea; }
.wc.close:hover{ background:#d0392b; color:#fff; }
.body{ flex:1; min-height:0; display:flex; }

/* ---- paged shell ---- */
.screen{ flex:1; min-height:0; width:100%; display:flex; flex-direction:column; }
.pageview{ flex:1; min-height:0; overflow-y:auto; overflow-x:hidden; padding:12px 22px 16px; }
.pageview::-webkit-scrollbar{ width:0; }
.page{ animation:rise .38s ease both; }
.phead{ font-family:'JetBrains Mono',monospace; font-size:11px; letter-spacing:.26em; color:var(--mut); margin:4px 0 5px; }
.psub{ font-size:12px; color:#8a949d; line-height:1.5; margin-bottom:14px; }

.nav{ display:flex; gap:4px; padding:8px 14px 12px; border-top:1px solid var(--line);
  background:linear-gradient(180deg, rgba(8,10,12,0), rgba(8,10,12,.55)); }
.navtab{ flex:1; display:flex; flex-direction:column; align-items:center; gap:4px; padding:8px 0;
  background:transparent; border:none; border-radius:12px; cursor:pointer; color:var(--mut); transition:.15s; }
.navtab:hover{ color:#aab4bd; background:#11161c; }
.navtab.on{ color:var(--acc); }
.navicon{ font-size:17px; line-height:1; }
.navtab.on .navicon{ text-shadow:0 0 10px var(--acc); }
.navlabel{ font-family:'JetBrains Mono',monospace; font-size:8.5px; letter-spacing:.16em; }

/* ---- WARP-style main toggle ---- */
.warpwrap{ display:flex; flex-direction:column; align-items:center; gap:16px; padding:26px 0 20px; }
.wbadge{ display:inline-flex; align-items:center; gap:7px; font-family:'JetBrains Mono',monospace;
  font-size:10px; letter-spacing:.24em; color:var(--mut); border:1px solid var(--line); border-radius:20px; padding:6px 14px; }
.wbadge.on{ color:var(--acc); border-color:rgba(55,242,166,.4); }
.wdot{ width:7px; height:7px; border-radius:50%; background:#39424b; flex:none; }
.wbadge.on .wdot, .stat.on .wdot{ background:var(--acc); box-shadow:0 0 8px var(--acc); animation:pulse 2s infinite; }
.warp{ width:230px; height:112px; border-radius:60px; border:1px solid var(--line); background:var(--card2);
  cursor:pointer; position:relative; display:block;
  transition:background .3s, border-color .3s, box-shadow .3s; }
.warp.on{ border-color:rgba(55,242,166,.5);
  background:linear-gradient(180deg, rgba(55,242,166,.18), rgba(55,242,166,.05));
  box-shadow:0 0 44px rgba(55,242,166,.28), inset 0 0 22px rgba(55,242,166,.07); }
.warp-knob{ position:absolute; top:7px; left:7px; width:96px; height:96px; border-radius:50%;
  background:#1b2128; color:var(--mut); display:flex; align-items:center; justify-content:center;
  transition:left .34s cubic-bezier(.34,1.4,.5,1), background .3s, color .3s, box-shadow .3s; }
/* geometrically-centered power glyph: currentColor box clipped by an svg mask */
.warp-knob::after{ content:""; width:44px; height:44px; background:currentColor;
  -webkit-mask:url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><g fill='none' stroke='white' stroke-width='2.2' stroke-linecap='round'><path d='M7.7 6.6 A6.6 6.6 0 1 0 16.3 6.6'/><path d='M12 3.2 L12 11.5'/></g></svg>") center/contain no-repeat;
  mask:url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><g fill='none' stroke='white' stroke-width='2.2' stroke-linecap='round'><path d='M7.7 6.6 A6.6 6.6 0 1 0 16.3 6.6'/><path d='M12 3.2 L12 11.5'/></g></svg>") center/contain no-repeat; }
.warp.on .warp-knob{ left:calc(100% - 7px - 96px);
  background:radial-gradient(circle at 35% 30%, #eafff5, var(--acc)); color:#04150d; box-shadow:0 0 26px var(--acc); }
/* toggled on but no engine attached — same slid-right position, red instead of green */
.wbadge.warn{ color:var(--warn); border-color:rgba(255,107,107,.45); }
.wbadge.warn .wdot{ background:var(--warn); box-shadow:0 0 8px var(--warn); animation:pulse 1.2s infinite; }
.warp.warn{ border-color:rgba(255,107,107,.55);
  background:linear-gradient(180deg, rgba(255,107,107,.16), rgba(255,107,107,.05));
  box-shadow:0 0 44px rgba(255,107,107,.26), inset 0 0 22px rgba(255,107,107,.07); }
.warp.warn .warp-knob{ left:calc(100% - 7px - 96px);
  background:radial-gradient(circle at 35% 30%, #ffe3e0, var(--warn)); color:#2a0d0d; box-shadow:0 0 26px var(--warn); }
.wtitle{ font-size:17px; font-weight:600; color:var(--txt); margin-top:2px; }
.wsub{ font-size:12px; color:#8a949d; text-align:center; line-height:1.5; max-width:300px; }
.mrow{ display:flex; justify-content:space-between; align-items:baseline; margin-bottom:12px; }
.mval{ font-family:'JetBrains Mono',monospace; font-weight:700; font-size:15px; color:var(--acc); }

/* ---- frequency bands ---- */
.bandlist{ display:flex; flex-direction:column; gap:7px; margin-top:14px; }
.band{ display:flex; align-items:center; justify-content:space-between; padding:10px 14px;
  border:1px solid var(--line); border-radius:12px; background:var(--card2); transition:.15s; }
.band.on{ border-color:rgba(55,242,166,.4); background:linear-gradient(180deg, rgba(55,242,166,.08), transparent); }
.bfreq{ font-family:'JetBrains Mono',monospace; font-weight:700; font-size:14px; color:var(--txt); }
.bnote{ font-family:'JetBrains Mono',monospace; font-size:9px; letter-spacing:.12em; color:var(--mut); margin-top:3px; }

/* ---- iOS-style switch ---- */
.switch{ width:46px; height:26px; border-radius:14px; border:1px solid var(--line); background:#12171d;
  cursor:pointer; padding:0; position:relative; transition:.18s; flex:none; }
.switch.on{ background:linear-gradient(180deg, var(--acc), var(--acc2)); border-color:transparent; }
.sknob{ position:absolute; top:2px; left:2px; width:20px; height:20px; border-radius:50%; background:#e9eef2;
  transition:transform .2s cubic-bezier(.34,1.4,.5,1); }
.switch.on .sknob{ transform:translateX(20px); background:#04150d; }

/* ---- parametric notches ---- */
.empty{ text-align:center; font-size:12px; color:var(--mut); padding:18px 0 6px; }
.notchlist{ display:flex; flex-direction:column; gap:9px; margin-top:14px; }
.notch{ border:1px solid var(--line); border-radius:13px; background:var(--card2); padding:12px 14px; }
.ntop{ display:flex; align-items:center; justify-content:space-between; margin-bottom:9px; }
.nfreq{ font-family:'JetBrains Mono',monospace; font-weight:700; font-size:16px; color:var(--acc); letter-spacing:.02em; }
.nrm{ width:24px; height:24px; border-radius:7px; border:1px solid var(--line); background:transparent;
  color:var(--mut); font-size:11px; cursor:pointer; transition:.13s; line-height:1; }
.nrm:hover{ background:#d0392b; color:#fff; border-color:transparent; }
.notch.off{ opacity:.5; }
.ntools{ display:flex; align-items:center; gap:7px; }
.nsw{ font-family:'JetBrains Mono',monospace; font-size:9px; letter-spacing:.12em; height:24px; min-width:40px;
  padding:0 9px; border-radius:7px; border:1px solid var(--line); background:transparent; color:var(--mut);
  cursor:pointer; transition:.13s; }
.nsw.on{ color:#04150d; background:var(--acc); border-color:transparent; box-shadow:0 0 10px rgba(55,242,166,.4); font-weight:700; }
.nsw:hover{ border-color:rgba(55,242,166,.5); }
.nsw.on:hover{ filter:brightness(1.08); }
.nrow{ display:flex; align-items:center; gap:10px; margin-top:8px; }
.nlab{ font-family:'JetBrains Mono',monospace; font-size:9px; letter-spacing:.18em; color:var(--mut); width:44px; flex:none; }
.nnum{ font-family:'JetBrains Mono',monospace; font-size:12px; color:var(--acc); width:56px; flex:none;
  text-align:right; background:#0c1116; border:1px solid var(--line); border-radius:7px; padding:5px 7px;
  outline:none; -moz-appearance:textfield; transition:border-color .13s; }
.nnum:focus{ border-color:var(--acc); }
.nnum::-webkit-inner-spin-button,.nnum::-webkit-outer-spin-button{ -webkit-appearance:none; margin:0; }
.nunit{ font-family:'JetBrains Mono',monospace; font-size:9px; letter-spacing:.1em; color:#3c454e; width:22px; flex:none; }
.mini{ -webkit-appearance:none; appearance:none; flex:1; height:5px; border-radius:4px; outline:none; cursor:pointer;
  background:linear-gradient(90deg, var(--acc) var(--pct), #1b2128 var(--pct)); }
.mini::-webkit-slider-thumb{ -webkit-appearance:none; width:15px; height:15px; border-radius:50%;
  background:#eafff5; border:2px solid var(--acc); box-shadow:0 0 8px rgba(55,242,166,.5); cursor:pointer; }

/* ---- live spectrum + suppression zones ---- */
.eq{ position:relative; height:176px; border:1px solid var(--line); border-radius:12px; overflow:hidden;
  background:radial-gradient(130% 100% at 50% 0%, rgba(55,242,166,.05), transparent 62%), #070a0d; }
.eqsvg{ position:absolute; inset:0; width:100%; height:100%; display:block; pointer-events:none; }
.vgrid{ stroke:rgba(150,170,185,.07); stroke-width:1; }
.sp{ fill:rgba(55,242,166,.18); stroke:var(--acc); stroke-width:1.6;
  filter:drop-shadow(0 0 4px rgba(55,242,166,.4)); }
.spin{ fill:none; stroke:rgba(200,212,222,.55); stroke-width:1.2; stroke-dasharray:4 3; }
.eqlegend{ position:absolute; top:8px; left:10px; display:flex; gap:12px; pointer-events:none; }
.eqlegend .lg{ font-family:'JetBrains Mono',monospace; font-size:8.5px; letter-spacing:.14em;
  display:flex; align-items:center; gap:5px; color:#7c8892; }
.eqlegend .lg::before{ content:""; width:12px; height:0; border-top:2px solid currentColor; }
.eqlegend .in{ color:#c8d4de; }
.eqlegend .in::before{ border-top-style:dashed; }
.eqlegend .out{ color:var(--acc); }
.bandov{ position:absolute; inset:0; pointer-events:none; }
.zline{ position:absolute; top:16px; bottom:0; width:0; border-left:1px dashed rgba(120,215,240,.5);
  transform:translateX(-.5px); }
.zline.off{ border-left:1px dashed rgba(150,170,185,.3); }
.zlabel{ position:absolute; top:3px; transform:translateX(-50%); font-family:'JetBrains Mono',monospace;
  font-size:9px; font-weight:700; color:#bfe8f4; letter-spacing:.02em; white-space:nowrap;
  text-shadow:0 0 6px rgba(0,0,0,.85); }
.zlabel.off{ color:#5a656e; font-weight:500; }
.eqaxis{ position:relative; height:13px; margin-top:7px; }
.eqaxis span{ position:absolute; transform:translateX(-50%); font-family:'JetBrains Mono',monospace;
  font-size:8.5px; color:#3c454e; }
.eqread{ font-family:'JetBrains Mono',monospace; font-size:10.5px; letter-spacing:.06em; color:var(--mut);
  text-align:center; margin-top:11px; min-height:14px; }

/* ---- settings ---- */
.setrow{ display:flex; align-items:center; justify-content:space-between; gap:12px; padding:13px 0;
  border-bottom:1px solid rgba(27,33,40,.55); }
.setrow:last-child{ border-bottom:none; padding-bottom:2px; }
.setrow:first-child{ padding-top:2px; }
.sett{ font-size:14px; color:var(--txt); font-weight:500; }
.setd{ font-family:'JetBrains Mono',monospace; font-size:10px; letter-spacing:.03em; color:var(--mut); margin-top:4px; }
.mselect{ -webkit-appearance:none; appearance:none; background:var(--card2); color:var(--txt);
  border:1px solid var(--line); border-radius:8px; padding:7px 10px;
  font-family:'JetBrains Mono',monospace; font-size:10px; letter-spacing:.03em;
  max-width:190px; text-overflow:ellipsis; cursor:pointer; outline:none; }
.mselect:hover{ border-color:var(--acc2); }
.mselect option{ background:var(--card); color:var(--txt); }
.stat{ display:inline-flex; align-items:center; gap:6px; font-family:'JetBrains Mono',monospace;
  font-size:10px; letter-spacing:.16em; color:var(--mut); }
.stat.on{ color:var(--acc); }
.sbtn{ font-family:'Chakra Petch'; font-size:11px; letter-spacing:.12em; color:var(--txt);
  background:var(--card2); border:1px solid var(--line); border-radius:9px; padding:9px 15px; cursor:pointer; transition:.15s; }
.sbtn:hover{ border-color:rgba(55,242,166,.45); color:var(--acc); }
.setabout{ color:#b9c2cc; font-size:12px; line-height:1.65; margin:9px 0 11px; }

/* ---- setup / GPU picker + download ---- */
.gpurow{ width:100%; display:flex; align-items:center; justify-content:space-between; gap:12px;
  margin-top:9px; padding:12px 14px; border:1px solid var(--line); border-radius:12px;
  background:var(--card2); color:var(--txt); cursor:pointer; text-align:left; transition:.14s; }
.gpurow:hover:not(:disabled){ border-color:rgba(55,242,166,.35); }
.gpurow.on{ border-color:rgba(55,242,166,.55); background:linear-gradient(180deg, rgba(55,242,166,.08), rgba(55,242,166,.02)); }
.gpurow:disabled{ opacity:.55; cursor:default; }
.gpuname{ font-size:13px; font-weight:600; color:var(--txt); }
.gpuarch{ font-family:'JetBrains Mono',monospace; font-size:10px; letter-spacing:.06em; color:var(--mut); margin-top:3px; }
.gpudot{ width:16px; height:16px; border-radius:50%; border:2px solid var(--line); flex:none; transition:.14s; }
.gpudot.on{ border-color:var(--acc); background:radial-gradient(circle at 50% 50%, var(--acc) 42%, transparent 46%);
  box-shadow:0 0 10px rgba(55,242,166,.5); }
.dlbtn{ width:100%; margin-top:14px; font-family:'Chakra Petch'; font-weight:600; font-size:14px; letter-spacing:.04em;
  color:#04150d; background:linear-gradient(180deg, var(--acc), var(--acc2)); border:none; border-radius:12px;
  padding:14px; cursor:pointer; box-shadow:0 0 22px rgba(55,242,166,.3); transition:.15s; }
.dlbtn:hover{ filter:brightness(1.06); }
.dlbar{ margin-top:14px; height:10px; border-radius:6px; background:#12171d; overflow:hidden; border:1px solid var(--line); }
.dlfill{ height:100%; background:linear-gradient(90deg, var(--acc2), var(--acc));
  box-shadow:0 0 12px rgba(55,242,166,.5); transition:width .15s linear; }
.dlnote{ font-family:'JetBrains Mono',monospace; font-size:10px; letter-spacing:.04em; color:var(--mut);
  margin-top:9px; text-align:center; }
.dlerr{ font-family:'JetBrains Mono',monospace; font-size:10.5px; line-height:1.5; color:var(--warn);
  background:rgba(255,107,107,.07); border:1px solid rgba(255,107,107,.25); border-radius:9px; padding:10px 12px; margin-top:11px; }
.root::before{ content:""; position:fixed; inset:0; pointer-events:none; opacity:.04;
  background-image:url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='120' height='120'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='.9' numOctaves='2'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E"); }

.brand{ font-weight:700; letter-spacing:.34em; text-transform:uppercase; }
.brand.sm{ font-size:15px; letter-spacing:.30em; }
.brand .dot{ color:var(--acc); text-shadow:0 0 10px var(--acc); }

/* ---- onboarding ---- */
.onb{ max-width:520px; margin:0 auto; animation:rise .5s ease both; }
.onb .brand{ font-size:30px; text-align:center; margin-top:8px; }
.tagline{ text-align:center; font-family:'JetBrains Mono',monospace; font-size:11px;
  letter-spacing:.34em; color:var(--mut); margin:8px 0 22px; }
.license{ background:var(--card); border:1px solid var(--line); border-radius:14px;
  padding:18px 20px; max-height:46vh; overflow:auto; }
.license p{ color:#b9c2cc; font-size:13px; line-height:1.6; margin:0 0 12px; }
.lhead{ color:var(--acc) !important; font-family:'JetBrains Mono',monospace; font-size:10px;
  letter-spacing:.2em; margin-top:6px !important; }
.lmut{ color:var(--mut) !important; font-size:11.5px !important; }
.agree{ width:100%; margin-top:18px; border:none; border-radius:12px; cursor:pointer;
  background:linear-gradient(180deg,var(--acc),var(--acc2)); color:#04150d; font-weight:700;
  font-family:'Chakra Petch'; letter-spacing:.16em; font-size:14px; padding:14px;
  box-shadow:0 0 24px rgba(55,242,166,.35); transition:transform .12s, box-shadow .2s; }
.agree:hover{ transform:translateY(-1px); box-shadow:0 0 34px rgba(55,242,166,.55); }

/* ---- deck ---- */
.deck{ max-width:440px; margin:0 auto; animation:rise .45s ease both; }
.top{ display:flex; justify-content:space-between; align-items:center; margin-bottom:16px; }
.sub{ font-family:'JetBrains Mono',monospace; font-size:9.5px; letter-spacing:.22em;
  color:var(--mut); margin-top:5px; }
.pill{ display:flex; align-items:center; gap:7px; font-family:'JetBrains Mono',monospace;
  font-size:10px; letter-spacing:.18em; color:var(--mut); background:var(--card);
  border:1px solid var(--line); border-radius:20px; padding:6px 12px; }
.pill.on{ color:var(--acc); border-color:rgba(55,242,166,.4); }
.led{ width:7px; height:7px; border-radius:50%; background:#39424b; }
.pill.on .led{ background:var(--acc); box-shadow:0 0 8px var(--acc); animation:pulse 2s infinite; }

.card{ background:linear-gradient(180deg,var(--card),var(--card2)); border:1px solid var(--line);
  border-radius:16px; padding:18px; margin-bottom:14px; }
.cardtop{ display:flex; justify-content:space-between; align-items:center; }
.label{ font-family:'JetBrains Mono',monospace; font-size:10px; letter-spacing:.2em; color:var(--mut); }
.label2{ font-family:'JetBrains Mono',monospace; font-size:10px; letter-spacing:.12em;
  color:var(--mut); margin:18px 0 10px; }
.pwr{ width:34px; height:34px; border-radius:10px; border:1px solid var(--line);
  background:var(--card2); color:var(--mut); font-size:15px; cursor:pointer; transition:.15s; }
.pwr.on{ color:var(--acc); border-color:rgba(55,242,166,.45); box-shadow:0 0 16px rgba(55,242,166,.28); }

.meter{ display:flex; align-items:baseline; gap:4px; margin:14px 0 16px; }
.big{ font-family:'JetBrains Mono',monospace; font-weight:700; font-size:54px; line-height:1;
  color:var(--txt); text-shadow:0 0 22px rgba(55,242,166,.25); font-variant-numeric:tabular-nums; }
.unit{ font-family:'JetBrains Mono',monospace; font-size:20px; color:var(--mut); }

.fader{ -webkit-appearance:none; appearance:none; width:100%; height:8px; border-radius:6px;
  background:linear-gradient(90deg, var(--acc) var(--pct), #1b2128 var(--pct)); outline:none; }
.fader:disabled{ filter:grayscale(.8) brightness(.7); }
.fader::-webkit-slider-thumb{ -webkit-appearance:none; width:20px; height:20px; border-radius:50%;
  background:#eafff5; border:3px solid var(--acc); box-shadow:0 0 12px var(--acc); cursor:pointer; }
.scale{ display:flex; justify-content:space-between; font-family:'JetBrains Mono',monospace;
  font-size:9px; letter-spacing:.18em; color:#3c454e; margin-top:9px; }

.spec{ display:flex; align-items:flex-end; gap:1px; height:96px; padding:8px 4px 0;
  background:radial-gradient(120% 100% at 50% 100%, rgba(55,242,166,.06), transparent 70%), #090c10;
  border:1px solid var(--line); border-radius:10px; }
.spec.tall{ height:132px; }
.bar{ flex:1; min-width:0; border-radius:2px 2px 0 0; cursor:pointer;
  background:linear-gradient(180deg, var(--acc), var(--acc2));
  box-shadow:0 0 6px rgba(55,242,166,.45); transition:height .06s linear; }
.bar.n{ background:linear-gradient(180deg, var(--warn), var(--warn2)); box-shadow:0 0 6px rgba(255,107,107,.5); }
.axis{ display:flex; justify-content:space-between; font-family:'JetBrains Mono',monospace;
  font-size:8.5px; color:#39424b; padding:5px 4px 0; }

.chips{ display:flex; flex-wrap:wrap; gap:6px; }
.chip{ font-family:'JetBrains Mono',monospace; font-size:11px; border:1px solid var(--line);
  background:var(--card2); color:#9aa6b1; border-radius:9px; padding:6px 11px; cursor:pointer; transition:.13s; }
.chip:hover{ border-color:#33414a; color:#cdd6df; }
.chip.on{ background:var(--warn); border-color:var(--warn); color:#260606; font-weight:700;
  box-shadow:0 0 12px rgba(255,107,107,.4); }
.quick{ display:flex; gap:8px; margin-top:11px; }
.q{ flex:1; font-family:'Chakra Petch'; letter-spacing:.06em; font-size:12px; border:1px solid var(--line);
  background:var(--card2); color:#c3ccd4; border-radius:9px; padding:9px; cursor:pointer; transition:.13s; }
.q:hover{ border-color:rgba(55,242,166,.4); color:var(--acc); }
.q.add{ flex:1.4; border-color:rgba(55,242,166,.35); color:var(--acc); }
.q.add:hover{ background:rgba(55,242,166,.1); }
.q:disabled{ opacity:.4; cursor:default; color:var(--mut); border-color:var(--line); background:var(--card2); }

.foot{ display:flex; align-items:center; gap:9px; font-family:'JetBrains Mono',monospace;
  font-size:10.5px; letter-spacing:.05em; color:var(--mut); margin-top:6px; padding:2px 4px; }
.fdot{ width:6px; height:6px; border-radius:50%; background:var(--acc); box-shadow:0 0 8px var(--acc); }
.foot{ } .foot :where(span):not(.fdot){ color:var(--acc); }

@keyframes rise{ from{ opacity:0; transform:translateY(10px);} to{ opacity:1; transform:none; } }
@keyframes pulse{ 0%,100%{ opacity:1;} 50%{ opacity:.35; } }
"#;
