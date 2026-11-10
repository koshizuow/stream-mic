'use strict';
// stream-mic's page, both layouts (a.html: hardware; b.html: scope; body[data-layout]). Everything it shows and sets is
// the engine's: GET/POST /api/state (settings), /api/stats (counters, once a second), /events (meters about 30 times
// a second with the scope's 5 ms points, and the devices), /api/autoset. Designed in the mock-ups
// (stream-mic-mock, 2026-11-07): dark, squared, English names for the processors.

const $ = (id) => document.getElementById(id);
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
const fmtDb = (v) => (v <= -99 ? '−∞' : (v < 0 ? '−' : v > 0 ? '+' : '') + Math.abs(v).toFixed(1));
const LAYOUT = document.body.dataset.layout;

// ------------------------------------------------------------------ languages: 繁中 and English
// The processors and their parameters are named in English in both (Denoise, Compressor, Threshold...: the words
// people meet in every plugin, user 2026-11-07); the rest follows the language. 厚薄 and 空間感 are our own knobs, so
// they are translated (Body, Space). tools/make_fonts.py takes the Chinese font's characters from this file.
const I18N = {
  zh: {
    running: '運作中', bypassed: '原音', offline: '沒有連上', loudness: '響度', send_lat: '送出延遲', mon_lat: '耳機', mon_none: '耳機不經過',
    edited: '改過', p_talk_omni: '說話（全指向）', p_talk_directional: '說話（指向）', p_sing: '唱歌',
    a_talk_omni: '筆電或全指向麥克風：房間的聲音收得多，補一點厚度。房間噪音大的話，Denoise 開一點。',
    a_talk_directional: '指向麥克風、離嘴巴近：聲音乾淨，不開 Denoise。',
    a_sing: '歌回：長音不會被當成噪音，有伴奏時加一點 Reverb。',
    k_body: '厚薄', k_space: '空間感',
    w_off: '關', w_light: '輕', w_mid: '中', w_strong: '強', w_flat: '原樣', w_thin: '薄 ', w_thick: '厚 ', floor_tick: '底噪',
    with_backing: '有伴奏', without_backing: '沒伴奏',
    sing_denoise: '歌回開 Denoise 最多只到「輕」：長音容易被當成噪音壓掉。',
    bypass: '聽原音', bypass_on: '原音中（再按一次回來）', bypass_on_short: '原音中', auto: '自動設定',
    space_hint: '按住空白鍵也可以暫時聽原音。', m_in: '進', m_out: '出', m_gr: 'GR', m_pc: 'PC',
    voice: '人聲 LUFS', m_pc_long: '電腦聲音 LUFS', chain: '處理鏈',
    chain_note_a: '點一段看細節，亮燈是開著；橘色條是它現在壓了多少（GR）',
    chain_note_b: '處理順序由左到右。點一段在下方看細節；亮燈是開著，橘色條是它現在壓了多少（GR）。',
    in_phones: '耳機也要', on: '開', eq_note: '拖曳圓點：Low Cut 左右移動頻率，其他上下調整增減。',
    verb_note: '風格依 TC-Helicon R1 的分類：各自的長度、音色、初期反射、預延遲；空間感旋鈕只調量。改了下面的數值就是自訂。', custom: '自訂',
    details: '細節', pick_stage: '在上面的處理鏈點一段。', drag_too: '；門檻也可以在上面的圖拖',
    pc: '電腦聲音', lb_on: '送進虛擬麥克風', lb_db: '送出的音量', pc_db: '耳機裡的音量',
    pc_note_linux: '遊戲、音樂、影片的輸出選「{sink}」。瀏覽器這類不能選輸出的程式，把系統的預設輸出設成它就好：耳機是這裡直接接上的，不會因此聽不到。',
    pc_note_win: '電腦裡所有程式的聲音（遊戲、音樂、瀏覽器）會自動收進來，不用改任何輸出。耳機裡照常由 Windows 播放。',
    mon: '監聽（耳機裡你自己的聲音）', mon_proc: '處理過的', mon_direct: 'Direct', mon_off: '關', mon_db: '你的聲音',
    mon_note_processed: '耳機裡是處理過的聲音（跳過會延遲的部分），加上電腦聲音。',
    mon_note_direct: '用錄音介面自己的 Direct Monitor（直接監聽）：0 延遲，但聽到的是沒處理過的聲音。這裡就不再送你的聲音到耳機，免得聽到兩次。',
    mon_note_off: '耳機裡沒有你自己的聲音。用喇叭時選這個：喇叭的聲音會被麥克風收進去。',
    devices: '裝置', mic: '麥克風', phones: '耳機', vmic: '虛擬麥克風',
    obs_note_linux: 'OBS、Discord 的輸入選「stream-mic」。', obs_note_win: 'OBS、Discord 的輸入選「CABLE Output (VB-Audio Virtual Cable)」。',
    dev_none: '找不到任何{what}，插上後會自動開始。', dev_missing: '之前選的{what}現在找不到（拔掉了？），先用系統預設的，插回去會自動換回來。',
    dev_mic: '麥克風', dev_out: '耳機', dev_out_any: '耳機或喇叭', dev_and: '和',
    no_cable: '沒有找到 VB-CABLE：虛擬麥克風要靠它。到 vb-cable.com 下載安裝，裝好後這裡會自動接上。',
    no_loopback: '收不到電腦聲音：需要 Windows 10 2004 以後的版本。',
    autostart: '開機時自動啟動（縮在系統匣，不開視窗）',
    tray_note: '關掉這個視窗後 stream-mic 會繼續在系統匣處理聲音，直播不會斷。要完全停止，在系統匣的圖示選「結束 stream-mic」。',
    floor: '房間底噪', floor_short: '房間底噪', gate_line: 'Gate', comp_line: 'Compressor', now: '現在', bypass_scope: '原音中：沒有處理',
    lg_in: '麥克風收到的', lg_out: '送出去的', lg_gate: 'Gate 門檻（拖）', lg_comp: 'Compressor 門檻（拖），頂端是壓了多少（GR）', lg_floor: '房間底噪',
    scope_label: '最近 10 秒的音量，門檻線可以拖',
    bal_ok: '電腦比人聲小 {d} dB，剛好', bal_loud: '電腦聲音偏大：只比人聲小 {d} dB', bal_soft: '電腦聲音偏小：比人聲小 {d} dB',
    bal_over: '電腦聲音比人聲大 {d} dB', bal_none: '沒有電腦聲音', bal_quiet: '還沒聽到你說話', bal_hint: '一般建議：電腦比人聲小 6–12 dB',
    loop: '偵測到回授：OBS 或 Discord 正在把 stream-mic 監聽到電腦聲音裡，電腦聲音先關掉了。請關掉那邊的監聽。',
    clip: '你的聲音加上電腦聲音太大，超過上限的部分已經自動圓滑壓住（不會破音）。常常出現的話，把「送出的音量」調低一點。',
    offline_msg: '沒有連上 stream-mic 的引擎：請確認它在執行，或重新整理。',
    settings: '設定', layout: '版面', layout_a: '設備', layout_b: '波形',
    language: '語言', layout_note: '兩種版面的功能一樣，只是排法不同。', sw_a: 'A 設備', sw_b: 'B 波形',
    prev_mode: '上一個模式', next_mode: '下一個模式',
    au_title: '自動設定', au_quiet: '1／2　請安靜 {s} 秒', au_quiet_t: '不要說話，先把音樂和影片停掉：量房間本身的噪音。',
    au_talk: '2／2　用平常直播的音量講話 {s} 秒', au_talk_t: '照平常的距離和音量，可以念這段：',
    au_say: '大家好，歡迎來到今天的直播！今天要玩的是一款新遊戲，在開始之前，先跟大家聊聊這個禮拜發生的事情。對了，記得幫我按個讚，有什麼想法也可以留言告訴我喔。',
    au_working: '計算中…', au_done: '設定好了', au_not: '沒有套用', au_error: '沒有完成', au_cancelled: '已取消', au_cancelled_t: '設定沒有改。',
    au_undone: '已復原', au_undone_t: '回到自動設定之前的設定。',
    au_result: 'Gate {gate} dB（房間底噪約 {floor} dB）、Compressor 從 {thr} dB 開始（講話時平均壓 {gr} dB）、Makeup {mk} dB。處理後的響度 {lufs} LUFS（目標 {target}）。只改了這三個，其他照舊；這個模式會記成「改過」。',
    au_novoice: '沒聽到講話聲，或環境噪音和講話一樣大，所以沒有套用。確認選的麥克風對不對，再做一次。',
    au_noisy: '房間噪音和你的聲音只差 {d} dB，Gate 分不太開：建議把 Denoise 打開一點，或靠近麥克風。',
    au_low: '收音太小，要補 {d} dB，噪音也會跟著變大：把麥克風或錄音介面的增益調大，再做一次。',
    au_loud: '講話時收音到了 {d} dBFS，快要破音：把麥克風或錄音介面的增益調小，再做一次。',
    au_hears_pc: '安靜的時候麥克風收到了電腦聲音：可能是錄音介面的 LOOPBACK 開著，或用的是喇叭。這樣電腦聲音會被送出兩次，請關掉 LOOPBACK 或改用耳機。',
    au_again: '再做一次', au_undo: '復原成之前的設定', au_cancel: '取消', au_close: '關閉',
  },
  en: {
    running: 'Running', bypassed: 'Bypassed', offline: 'Not connected', loudness: 'Loudness', send_lat: 'Send latency', mon_lat: 'Phones', mon_none: 'Phones: off',
    edited: 'edited', p_talk_omni: 'Talk (omni)', p_talk_directional: 'Talk (directional)', p_sing: 'Singing',
    a_talk_omni: 'Laptop or omni mic: it hears much of the room; a little more body. In a noisy room, turn Denoise up a little.',
    a_talk_directional: 'Directional mic close to the mouth: a clean voice, Denoise off.',
    a_sing: 'Singing: long notes are not taken for noise; a little Reverb with a backing track.',
    k_body: 'Body', k_space: 'Space',
    w_off: 'Off', w_light: 'Light', w_mid: 'Medium', w_strong: 'Strong', w_flat: 'Flat', w_thin: 'Thin ', w_thick: 'Warm ', floor_tick: 'floor',
    with_backing: 'with backing', without_backing: 'without',
    sing_denoise: 'Singing: Denoise goes up to Light only; it takes long notes for noise.',
    bypass: 'Bypass', bypass_on: 'Bypassed (click to return)', bypass_on_short: 'Bypassed', auto: 'Auto setup',
    space_hint: 'Hold Space to hear the raw voice for a moment.', m_in: 'In', m_out: 'Out', m_gr: 'GR', m_pc: 'PC',
    voice: 'Voice LUFS', m_pc_long: 'Computer LUFS', chain: 'Chain',
    chain_note_a: 'Click a stage for its details. Lit: on; the amber bar: its gain reduction now',
    chain_note_b: 'Processed left to right. Click a stage for its details below. Lit: on; the amber bar: its gain reduction now.',
    in_phones: 'In headphones', on: 'On', eq_note: 'Drag a point: Low Cut left and right for its frequency, the others up and down.',
    verb_note: 'Styles after TC-Helicon R1: each its own length, tone, early reflections and pre-delay; the Space knob sets only how much. Changing a value below makes it custom.', custom: 'Custom',
    details: 'Details', pick_stage: 'Click a stage in the chain above.', drag_too: '; the threshold can be dragged in the scope too',
    pc: 'Computer audio', lb_on: 'Send to the virtual mic', lb_db: 'Send level', pc_db: 'In headphones',
    pc_note_linux: 'Set games, music and videos to play to “{sink}”. For programs without that choice (a browser), make it the system’s default output: the headphones are connected here, so you still hear everything.',
    pc_note_win: 'Every program’s sound (games, music, a browser) comes in by itself; nothing to change. Windows plays it in your headphones as usual.',
    mon: 'Monitor (your voice in the headphones)', mon_proc: 'Processed', mon_direct: 'Direct', mon_off: 'Off', mon_db: 'Your voice',
    mon_note_processed: 'The headphones get the processed voice (but the stages that delay it), with the computer audio.',
    mon_note_direct: 'Your interface’s own direct monitor: no delay, but the raw voice. Your voice is not sent to the headphones from here as well.',
    mon_note_off: 'No voice in the headphones. Choose this with speakers: the mic would hear them.',
    devices: 'Devices', mic: 'Microphone', phones: 'Headphones', vmic: 'Virtual mic',
    obs_note_linux: 'In OBS and Discord, choose “stream-mic” as the input.', obs_note_win: 'In OBS and Discord, choose “CABLE Output (VB-Audio Virtual Cable)” as the input.',
    dev_none: 'No {what} found; it starts by itself once one is plugged in.', dev_missing: 'The {what} chosen before is not there (unplugged?): the system’s default for now, and back to it once it returns.',
    dev_mic: 'microphone', dev_out: 'headphones', dev_out_any: 'headphones or speakers', dev_and: ' and ',
    no_cable: 'VB-CABLE not found: the virtual mic needs it. Install it from vb-cable.com; it is taken up here by itself.',
    no_loopback: 'No computer audio: Windows 10 version 2004 or later is needed.',
    autostart: 'Start with the computer (in the tray, no window)',
    tray_note: 'Closing this window leaves stream-mic working in the tray, the stream goes on. To stop it, choose “Quit stream-mic” on the tray icon.',
    floor: 'Room noise floor', floor_short: 'Noise floor', gate_line: 'Gate', comp_line: 'Compressor', now: 'now', bypass_scope: 'Bypassed: no processing',
    lg_in: 'Mic in', lg_out: 'Sent', lg_gate: 'Gate threshold (drag)', lg_comp: 'Compressor threshold (drag); gain reduction at the top', lg_floor: 'Room noise floor',
    scope_label: 'The level over the last 10 s; the thresholds can be dragged',
    bal_ok: 'Computer {d} dB under the voice: good', bal_loud: 'Computer loud: only {d} dB under the voice', bal_soft: 'Computer quiet: {d} dB under the voice',
    bal_over: 'Computer {d} dB over the voice', bal_none: 'No computer audio', bal_quiet: 'Not hearing you yet', bal_hint: 'The usual advice: computer 6–12 dB under the voice',
    loop: 'Feedback: OBS or Discord is monitoring stream-mic into the computer audio, so the computer audio is off for now. Turn that monitoring off.',
    clip: 'Your voice and the computer audio together went over the top; it was rounded off (no clipping). If it keeps happening, lower the send level.',
    offline_msg: 'Not connected to stream-mic’s engine: check it is running, or reload.',
    settings: 'Settings', layout: 'Layout', layout_a: 'Hardware', layout_b: 'Scope',
    language: 'Language', layout_note: 'Both layouts do the same; only the arrangement differs.', sw_a: 'A Hardware', sw_b: 'B Scope',
    prev_mode: 'Previous mode', next_mode: 'Next mode',
    au_title: 'Auto setup', au_quiet: '1/2  Quiet, please: {s} s', au_quiet_t: 'Do not talk; stop music and videos: this measures the room itself.',
    au_talk: '2/2  Talk as you do on stream: {s} s', au_talk_t: 'At your usual distance and level; you can read this:',
    au_say: 'Hi everyone, welcome to today’s stream! We are playing a new game today, but first let me tell you what happened this week. And remember to hit like, and tell me what you think in the chat.',
    au_working: 'Working it out…', au_done: 'Done', au_not: 'Not applied', au_error: 'Not finished', au_cancelled: 'Cancelled', au_cancelled_t: 'Nothing was changed.',
    au_undone: 'Undone', au_undone_t: 'Back to the settings from before.',
    au_result: 'Gate {gate} dB (room noise about {floor} dB), Compressor from {thr} dB (about {gr} dB of reduction while talking), Makeup {mk} dB. Loudness after: {lufs} LUFS (target {target}). Only these three changed; this mode now shows as edited.',
    au_novoice: 'No voice heard, or the room is as loud as the voice: nothing applied. Check the microphone chosen and try again.',
    au_noisy: 'The room is only {d} dB under your voice; the Gate cannot tell them apart well. Turn Denoise up a little, or get closer to the mic.',
    au_low: 'The mic is quiet: {d} dB had to be added, and the noise with it. Turn the mic’s or interface’s gain up and try again.',
    au_loud: 'Talking reached {d} dBFS, close to clipping. Turn the mic’s or interface’s gain down and try again.',
    au_hears_pc: 'The mic heard the computer audio while it was quiet: the interface’s LOOPBACK may be on, or these are speakers. The computer audio would go out twice; turn LOOPBACK off or use headphones.',
    au_again: 'Again', au_undo: 'Back to before', au_cancel: 'Cancel', au_close: 'Close',
  },
};
let LANG = /^zh/i.test(navigator.language || '') ? 'zh' : 'en';
const t = (k, v) => {
  let s = I18N[LANG][k] ?? I18N.zh[k] ?? k;
  if (v) for (const [a, b] of Object.entries(v)) s = s.replace('{' + a + '}', b);
  return s;
};
function applyI18n(root = document) {
  document.documentElement.lang = LANG === 'zh' ? 'zh-Hant' : 'en';
  for (const el of root.querySelectorAll('[data-i18n]')) el.textContent = t(el.dataset.i18n);
  for (const el of root.querySelectorAll('[data-i18n-aria]')) el.setAttribute('aria-label', t(el.dataset.i18nAria));
}

// ------------------------------------------------------------------ the engine: its state, changes, counters
let S = null;          // GET /api/state
let ST = {};           // GET /api/stats
let D = null;          // the devices (event "devices")
let online = false;
const P = () => S.preset;
// the reverb's styles (crates/chain/src/preset.rs VERB_STYLES, TC-Helicon R1's names and order)
const VERB_STYLES = [['hall', 'Hall'], ['plate', 'Plate'], ['theatre', 'Theatre'], ['club', 'Club'], ['room', 'Room'], ['studio', 'Studio'], ['ambience', 'Ambience'], ['arena', 'Arena']];
const verbStyleName = () => (VERB_STYLES.find(([v]) => v === P().verb.style) || [0, t('custom')])[1];
const sing = () => !!P().follow_music;
const win = () => S && S.platform === 'windows';
const holding = new Set();   // controls being dragged: the answer does not move them

// a change goes as just the part changed; the answer is the whole state, shown again. While one is on its way the
// next ones are merged and sent after it
let inflight = false, queued = null;
function deep(a, b) { for (const [k, v] of Object.entries(b)) { if (v && typeof v === 'object' && !Array.isArray(v) && a[k] && typeof a[k] === 'object') deep(a[k], v); else a[k] = v; } return a; }
function post(change, then) {
  if (inflight) { queued = queued ? deep(queued, change) : change; return; }
  inflight = true;
  fetch('/api/state', { method: 'POST', body: JSON.stringify(change) }).then((r) => r.json()).then((st) => { S = st; render(); if (then) then(); })
    .catch(() => {}).finally(() => { inflight = false; if (queued) { const q = queued; queued = null; post(q); } });
}
// a slider or a knob being dragged: at most every 60 ms, and once more when let go
function throttled(send) {
  let tmr = 0, last = null;
  const go = () => { tmr = 0; if (last !== null) { send(last); last = null; } };
  return { move: (v) => { last = v; if (!tmr) tmr = setTimeout(go, 60); }, end: (v) => { last = v; clearTimeout(tmr); go(); } };
}

// ------------------------------------------------------------------ the knobs, the modes, the stages
const MODE_ICON = { 'talk-omni': '◎', 'talk-directional': '◉', sing: '♪' };
const modeName = (b) => (I18N[LANG]['p_' + b.name.replace(/-/g, '_')] ? t('p_' + b.name.replace(/-/g, '_')) : b.label);
const modeAbout = (b) => (I18N[LANG]['a_' + b.name.replace(/-/g, '_')] ? t('a_' + b.name.replace(/-/g, '_')) : b.about);
const KNOBS = [
  { id: 'denoise', name: () => 'Denoise', min: 0, max: () => (sing() ? 30 : 100), word: (v) => (v === 0 ? t('w_off') : v < 34 ? t('w_light') : v < 67 ? t('w_mid') : t('w_strong')) },
  { id: 'body', name: () => t('k_body'), min: -100, max: () => 100, word: (v) => (v === 0 ? t('w_flat') : (v < 0 ? t('w_thin') : t('w_thick')) + Math.abs(Math.round(v / 10))) },
  { id: 'space', name: () => t('k_space'), min: 0, max: () => 100, word: (v) => (v === 0 ? t('w_off') : verbStyleName() + ' · ' + (sing() ? t('with_backing') + ' ' + Math.round(P().verb.mix) + '% · ' + t('without_backing') + ' ' + Math.round(P().verb.mix_dry) + '%' : Math.round(v) + '')) },
  { id: 'sib', name: () => 'De-ess', min: 0, max: () => 100, word: (v) => (v === 0 ? t('w_off') : Math.round(v) + '') },
  { id: 'gate', name: () => 'Gate', min: -80, max: () => -20, word: (v) => (v <= -80 ? t('w_off') : Math.round(v) + ' dB') },
];
// each stage and its parameters, the engine's names and ranges (crates/chain/src/preset.rs)
const STAGES = [
  { id: 'denoise', name: 'Denoise', params: [['limit', 'Reduction', 0, 40, 1, 'dB']] },
  { id: 'eq', name: 'EQ', params: [['hp', 'Low Cut', 40, 200, 5, 'Hz'], ['low', 'Low (200 Hz)', -6, 6, 0.5, 'dB'], ['pres', 'Presence (3 kHz)', -6, 6, 0.5, 'dB'], ['air', 'Air (10 kHz)', -6, 6, 0.5, 'dB']] },
  { id: 'deess', name: 'De-esser', params: [['thr', 'Threshold', -10, 20, 0.5, 'dB'], ['freq', 'Frequency', 4000, 10000, 100, 'Hz'], ['max', 'Range', 3, 12, 1, 'dB']] },
  { id: 'exp', name: 'Gate', params: [['thr', 'Threshold', -80, -20, 1, 'dBFS'], ['ratio', 'Ratio 1:', 1.2, 4, 0.1, ''], ['range', 'Range', 3, 20, 1, 'dB'], ['hold', 'Hold', 0, 300, 10, 'ms'], ['rel', 'Release', 50, 1000, 10, 'ms']] },
  { id: 'comp', name: 'Compressor', params: [['thr', 'Threshold', -40, -6, 0.5, 'dBFS'], ['ratio', 'Ratio', 1, 8, 0.1, ':1'], ['knee', 'Knee', 0, 12, 0.5, 'dB'], ['att', 'Attack', 1, 40, 1, 'ms'], ['rel', 'Release', 30, 400, 5, 'ms'], ['makeup', 'Makeup', 0, 18, 0.5, 'dB']] },
  { id: 'verb', name: 'Reverb', params: [['mix', 'Mix', 0, 40, 1, '%'], ['mix_dry', 'Mix (no backing)', 0, 40, 1, '%', 'sing'], ['follow', 'Follow', 0.3, 4, 0.1, 's', 'sing'], ['size', 'Decay', 0.2, 2.5, 0.05, 's'], ['pre', 'Pre-delay', 0, 60, 1, 'ms'], ['lowcut', 'Low Cut', 100, 500, 10, 'Hz'], ['damp', 'Damping', 2000, 12000, 100, 'Hz'], ['er', 'Early reflections', 0, 1, 0.05, ''], ['er_ms', 'Reflections spread', 5, 100, 1, 'ms']] },
  { id: 'lim', name: 'Limiter', params: [['ceil', 'Ceiling (true peak)', -6, 0, 0.1, 'dBTP'], ['rel', 'Release', 20, 200, 5, 'ms']] },
];
const fmtP = (v, step) => (step < 1 ? (+v).toFixed(step < 0.1 ? 2 : 1) : String(Math.round(v)));
const stageLatency = (id) => (id === 'denoise' ? ST.denoise_ms || 20 : id === 'lim' ? ST.send_limiter_ms || 2.1 : 0);
const msText = (ms) => (ms ? (Math.round(ms * 10) / 10).toFixed(ms % 1 ? 1 : 0) : '0') + ' ms';
function setKnob(id, v) {
  P().knobs[id] = v;
  if (!knobSend[id]) knobSend[id] = throttled((x) => post({ knobs: { [id]: x } }));
  return knobSend[id];
}
const knobSend = {};

// ------------------------------------------------------------------ the levels: meters, the scope's history
// each of in, out (the voice through the chain) and pc (the computer's sound as sent): the peak falling 20 dB a second
// (broadcast meters' pace), the mean over 300 ms (the bar's body), a peak held 1.5 s, a clip lamp lit until clicked
const meter = () => ({ pk: -90, rms: -90, ms: 0, hold: -90, holdT: 0, clip: false });
const M = { in: meter(), out: meter(), pc: meter(), lufs: -120, pcLufs: -120, gr: 0, grGate: 0, grComp: 0, grLim: 0, floor: -120, loop: false };
function feed(m, pk, rms, dt) {
  const p = Math.pow(10, rms / 10);
  m.ms += (p - m.ms) * (1 - Math.exp(-dt / 0.3));
  m.rms = 10 * Math.log10(m.ms + 1e-12);
  m.pk = Math.max(pk, m.pk - 20 * dt);
  if (pk >= m.hold || m.holdT <= 0) { m.hold = pk; m.holdT = 1.5; } else m.holdT -= dt;
  if (pk > -0.5) m.clip = true;
}
// the scope: (mic peak, voice peak, reduction) every 5 ms, the last 10 s
const RATE = 200, HN = RATE * 10;
const H = { inp: new Float32Array(HN).fill(-120), out: new Float32Array(HN).fill(-120), gr: new Float32Array(HN), head: 0 };
let lastMeters = performance.now();
function onMeters(m) {
  const now = performance.now(), dt = Math.min(0.2, (now - lastMeters) / 1000);
  lastMeters = now;
  // rms is a power over the event's window: 20 log10(rms) already (dBFS); peaks likewise
  feed(M.in, m.in_pk, m.in_rms, dt);
  feed(M.out, m.out_pk, m.out_rms, dt);
  feed(M.pc, m.pc_pk ?? -120, m.pc_rms ?? -120, dt);
  M.lufs = m.lufs; M.pcLufs = m.pc_lufs ?? -120;
  M.gr = -(m.gr || 0); M.grGate = m.gr_gate || 0; M.grComp = m.gr_comp || 0; M.grLim = m.gr_lim || 0;
  if (m.floor > -110) M.floor = m.floor;
  M.loop = !!m.loop;
  for (const p of m.hist || []) { H.inp[H.head] = p[0]; H.out[H.head] = S && S.bypass ? p[0] : p[1]; H.gr[H.head] = p[2]; H.head = (H.head + 1) % HN; }
}

// ------------------------------------------------------------------ a canvas sized to its box, sharp on HiDPI
function fitCanvas(c) {
  const r = c.getBoundingClientRect(), d = devicePixelRatio || 1;
  const w = Math.max(1, Math.round(r.width * d)), h = Math.max(1, Math.round(r.height * d));
  if (c.width !== w || c.height !== h) { c.width = w; c.height = h; }
  const g = c.getContext('2d');
  g.setTransform(d, 0, 0, d, 0, 0);
  return [g, r.width, r.height];
}
const css = (v) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();
// the canvases' fonts: numbers in the mono face, words in the interface's
const MONO = '"JetBrains Mono", ui-monospace, "Noto Sans TC", monospace', UIF = '"Inter", "Noto Sans TC", system-ui, "Microsoft JhengHei", sans-serif';

// ------------------------------------------------------------------ meters, the way consoles draw them
// In, Out (the voice through the chain) and PC (the computer's sound) side by side, GR beside them. A scale that
// opens out near the top (-6 to 0 gets room; under -40 little); each bar a body at its mean (300 ms) with the moment's
// peak lighter above it, a held peak with its number on top (red past -0.5 dB), colour by zone (its own, amber from
// -6, red from -1), faint segments like an LED bar.
const PC_COLOR = '#a98bf5';
function meterPos(db) { return 1 - Math.pow(clamp(-db, 0, 60) / 60, 0.7); }
function meterY(db, top, h) { return top + h * (1 - meterPos(db)); }
function drawMeters(c) {
  const [g, w, h] = fitCanvas(c);
  g.clearRect(0, 0, w, h);
  const top = 18, bot = 20, mh = h - top - bot, Y = (db) => Math.round(meterY(db, top, mh));
  const bars = [
    { m: M.in, label: t('m_in'), color: '#a3aab4' },
    { m: M.out, label: t('m_out'), color: css('--accent'), lufs: M.lufs },
    { m: M.pc, label: t('m_pc'), color: PC_COLOR, lufs: S && S.loopback ? M.pcLufs : null },
  ];
  // the voice's group (In, Out, its GR), then the computer's sound apart: the chain turns down only the voice
  const SCALE = 20, GAP = 4, OGAP = 8, GRW = 8, SEP = 20, TRI = 7;
  const bw = Math.max(9, Math.min(17, (w - SCALE - 6 - GAP - OGAP - GRW - SEP - TRI - 4) / 3));
  const barsW = bw + GAP + bw + OGAP + GRW + SEP + bw + TRI;
  const bx = Math.max(SCALE + 6, Math.round((w - barsW) / 2) + 6);
  const ox = bx + bw + GAP, gx = ox + bw + OGAP, px = gx + GRW + SEP, xs = [bx, ox, px];
  // the scale: numbers where they are read, short ticks between
  g.textAlign = 'right'; g.textBaseline = 'middle'; g.font = '9.5px ' + MONO;
  for (let d = 0; d >= -60; d -= 1) {
    const major = [0, -6, -12, -18, -24, -40, -60].includes(d), minor = d > -24 ? d % 3 === 0 : d % 10 === 0;
    if (!major && !minor) continue;
    const y = Y(d);
    if (major) { g.fillStyle = css('--faint'); g.fillText(d === 0 ? '0' : '−' + -d, bx - 7, y); }
    g.fillStyle = major ? 'rgba(255,255,255,.18)' : 'rgba(255,255,255,.09)'; g.fillRect(bx - 4, y, major ? 3 : 2, 1);
  }
  // LED bars: every segment drawn, unlit ones dim; lit to the mean, half lit to the peak, the top lit one glowing
  const SEG = 3, zone = (db, col) => (db >= -1 ? css('--hot') : db >= -6 ? css('--comp') : col);
  bars.forEach((b, i) => {
    const x = xs[i], m = b.m;
    g.fillStyle = '#07090b'; g.fillRect(x - 1, top - 1, bw + 2, mh + 2);
    const yr = Y(m.rms), yp = Y(m.pk);
    let topLit = null;
    for (let y = top + mh - SEG; y >= top; y -= SEG) {
      const db = -60 * Math.pow(1 - (top + mh - y) / mh, 1 / 0.7);
      const col = zone(db, b.color);
      const lit = y >= yr ? 1 : y >= yp ? 0.45 : 0;
      g.globalAlpha = lit ? lit : 0.09;
      g.fillStyle = col; g.fillRect(x, y, bw, SEG - 1);
      if (lit === 1) topLit = [y, col];
    }
    g.globalAlpha = 1;
    if (topLit) {
      g.save(); g.shadowColor = topLit[1]; g.shadowBlur = 8; g.fillStyle = topLit[1]; g.fillRect(x, topLit[0], bw, SEG - 1); g.restore();
      g.fillStyle = 'rgba(255,255,255,.35)'; g.fillRect(x, topLit[0], bw, 1);
    }
    // the held peak and its number (whole dB: a decimal does not fit over a bar); a clip lamp above, lit until
    // clicked
    g.fillStyle = '#eef1f4'; g.fillRect(x, Y(m.hold) - 1, bw, 2);
    g.fillStyle = m.clip ? css('--hot') : '#2a2f36'; g.fillRect(x, top - 6, bw, 3);
    g.fillStyle = m.clip ? css('--hot') : css('--muted'); g.font = '8.5px ' + MONO; g.textAlign = 'center'; g.textBaseline = 'middle';
    const hv = Math.round(m.hold);
    g.fillText(m.hold <= -60 ? '−∞' : (hv < 0 ? '−' : '') + Math.abs(hv), x + bw / 2, 6);
    // its loudness now (LUFS): a triangle on its right
    if (b.lufs != null && b.lufs > -60) {
      const ly = Y(b.lufs), tx = x + bw + 1;
      g.fillStyle = i === 2 ? PC_COLOR : css('--accent');
      g.beginPath(); g.moveTo(tx, ly); g.lineTo(tx + TRI - 1, ly - 4); g.lineTo(tx + TRI - 1, ly + 4); g.closePath(); g.fill();
    }
    g.fillStyle = i === 2 ? PC_COLOR : css('--muted'); g.font = '10.5px ' + UIF; g.textBaseline = 'middle';
    g.fillText(b.label, x + bw / 2, h - 9);
  });
  // the ceiling over the voice's bars, and the voice's target (-18 LUFS, -16 to -20) as a bracket left of Out
  g.fillStyle = css('--hot'); g.fillRect(bx, Y(-1), ox + bw - bx, 1);
  g.fillStyle = css('--accent'); g.globalAlpha = 0.9;
  g.fillRect(ox - 3, Y(-16), 2, Y(-20) - Y(-16)); g.fillRect(ox - 3, Y(-16), 3, 1); g.fillRect(ox - 3, Y(-20), 3, 1);
  g.globalAlpha = 1;
  // gain reduction: from the top, amber, its own scale (0 to -24 dB, numbers on its right)
  const gyy = (d) => top + Math.round((mh * clamp(d, 0, 24)) / 24);
  g.fillStyle = '#07090b'; g.fillRect(gx - 1, top - 1, GRW + 2, mh + 2);
  for (let y = top; y < top + mh; y += SEG) {
    const on = y < gyy(-M.gr);
    g.globalAlpha = on ? 1 : 0.09; g.fillStyle = css('--comp'); g.fillRect(gx, y, GRW, SEG - 1);
  }
  g.globalAlpha = 1;
  g.font = '8.5px ' + MONO; g.textAlign = 'left'; g.fillStyle = css('--faint');
  for (const d of [3, 6, 12, 24]) { const y = gyy(d); g.fillRect(gx + GRW + 1, y, 2, 1); g.fillText(d === 24 ? '24' : d, gx + GRW + 4, y); }
  g.fillStyle = css('--muted'); g.textAlign = 'center'; g.font = '10.5px ' + UIF; g.fillText(t('m_gr'), gx + GRW / 2, h - 9);
  c.style.cursor = M.in.clip || M.out.clip || M.pc.clip ? 'pointer' : 'default';
  if (!c.dataset.clipWired) { c.dataset.clipWired = 1; c.addEventListener('click', () => { M.in.clip = M.out.clip = M.pc.clip = false; }); }
}

// the voice against the computer's sound: their loudness, and how far apart (6 to 12 dB under the voice is the usual
// advice for games and music on a stream: a rule of thumb)
function balanceHtml() {
  if (!S) return '';
  const d = M.lufs - M.pcLufs;
  const pcOn = S && S.loopback && M.pcLufs > -60;
  let cls = 'good', msg;
  // no voice yet (or a long pause): nothing to compare, and no alarm (the engine will gate it as BS.1770 does)
  if (M.lufs < -50) { cls = ''; msg = t('bal_quiet'); }
  else if (!pcOn) { cls = ''; msg = t('bal_none'); }
  else if (d < 0) { cls = 'bad'; msg = t('bal_over').replace('{d}', (-d).toFixed(0)); }
  else if (d < 6) { cls = 'warn'; msg = t('bal_loud').replace('{d}', d.toFixed(0)); }
  else if (d > 14) { cls = 'warn'; msg = t('bal_soft').replace('{d}', d.toFixed(0)); }
  else msg = t('bal_ok').replace('{d}', d.toFixed(0));
  const v = (x) => (x <= -60 ? '−∞' : fmtDb(x));
  // the gap as a gauge: 0 to 20 dB under the voice, 6 to 12 green, the pointer at now (left of 0: louder than the voice)
  const gx = (dd) => 4 + (clamp(dd, -2, 20) + 2) / 22 * 172;
  const gauge = '<svg class="gauge" viewBox="0 0 180 25" aria-hidden="true">' +
    '<rect x="4" y="9" width="172" height="4" fill="#0b0d10" stroke="#2a2f37"/>' +
    '<rect x="' + gx(6) + '" y="9" width="' + (gx(12) - gx(6)) + '" height="4" fill="rgba(79,214,180,.55)"/>' +
    [0, 6, 12, 20].map((v) => '<line x1="' + gx(v) + '" y1="14" x2="' + gx(v) + '" y2="17" stroke="#5b626d"/><text x="' + gx(v) + '" y="24" font-size="8" fill="#6b737e" text-anchor="middle" font-family="JetBrains Mono, monospace">' + v + '</text>').join('') +
    (pcOn && M.lufs >= -50 ? '<path d="M' + gx(d) + ' 8 l-4 -6 h8 z" fill="' + (cls === 'good' ? '#4fd6b4' : cls === 'bad' ? '#ff5c5c' : '#f2a541') + '"/>' : '') +
    '</svg>';
  return '<div class="bal"><div><small>' + t('voice') + '</small><b class="num vo">' + v(M.lufs) + '</b></div>' +
    '<div><small>' + t('m_pc_long') + '</small><b class="num pc">' + v(pcOn ? M.pcLufs : -99) + '</b></div></div>' +
    gauge + '<p class="balmsg ' + cls + '">' + msg + '</p>';
}


// ------------------------------------------------------------------ a knob, drawn: ticks, its arc, a knurled skirt that turns, a lit cap
function knobSvg(v, k) {
  const kmax = k.max();
  k = { ...k, max: kmax };
  const a0 = -135, a1 = 135, f = (clamp(v, k.min, k.max) - k.min) / (k.max - k.min), ang = a0 + f * (a1 - a0);
  const r = 41, cx = 52, cy = 52;
  const pt = (a, rr) => [(cx + rr * Math.sin(a * Math.PI / 180)).toFixed(2), (cy - rr * Math.cos(a * Math.PI / 180)).toFixed(2)];
  const arc = (from, to) => { const [x0, y0] = pt(from, r), [x1, y1] = pt(to, r); return 'M' + x0 + ' ' + y0 + ' A' + r + ' ' + r + ' 0 ' + (to - from > 180 ? 1 : 0) + ' 1 ' + x1 + ' ' + y1; };
  // bipolar knobs light from the middle
  const from = k.min < 0 && k.max > 0 ? a0 + ((0 - k.min) / (k.max - k.min)) * (a1 - a0) : a0;
  const [lo, hi] = ang < from ? [ang, from] : [from, ang];
  const color = k.id === 'gate' ? 'var(--gate)' : 'var(--accent)';
  // the scale: eleven ticks outside the arc, the ends longer
  let ticks = '';
  for (let i = 0; i <= 10; i++) {
    const a = a0 + (i / 10) * (a1 - a0), end = i === 0 || i === 10 || i === 5;
    const [x0, y0] = pt(a, 46), [x1, y1] = pt(a, end ? 50.5 : 49);
    ticks += '<line x1="' + x0 + '" y1="' + y0 + '" x2="' + x1 + '" y2="' + y1 + '" stroke="' + (end ? '#5d6570' : '#3a4049') + '" stroke-width="1.2"/>';
  }
  // the gate: the room's noise floor as a tick across its arc
  if (k.id === 'gate' && M.floor > -110) {
    const fa = a0 + ((clamp(M.floor, k.min, k.max) - k.min) / (k.max - k.min)) * (a1 - a0);
    const [t0x, t0y] = pt(fa, r - 5), [t1x, t1y] = pt(fa, r + 5);
    ticks += '<line x1="' + t0x + '" y1="' + t0y + '" x2="' + t1x + '" y2="' + t1y + '" stroke="#c9d6ea" stroke-width="2"/>';
  }
  // the knurled skirt turns with the knob; the light does not
  let knurl = '';
  for (let i = 0; i < 40; i++) {
    const a = (i / 40) * 360, [x0, y0] = pt(a, 26.5), [x1, y1] = pt(a, 30.5);
    knurl += '<line x1="' + x0 + '" y1="' + y0 + '" x2="' + x1 + '" y2="' + y1 + '"/>';
  }
  const [px0, py0] = pt(0, 9), [px1, py1] = pt(0, 22);
  return '<svg viewBox="0 0 104 104" aria-hidden="true">' + ticks +
    '<path d="' + arc(a0, a1) + '" stroke="#20252c" stroke-width="4" fill="none" stroke-linecap="butt"/>' +
    (hi - lo > 0.5 ? '<path d="' + arc(lo, hi) + '" stroke="' + color + '" stroke-width="4" fill="none" stroke-linecap="butt"/>' : '') +
    '<circle cx="52" cy="55" r="31" fill="#000" opacity=".55" filter="url(#kShadow)"/>' +
    '<circle cx="52" cy="52" r="31" fill="url(#kSkirt)"/>' +
    '<g transform="rotate(' + ang.toFixed(1) + ' 52 52)"><g stroke="#0b0d10" stroke-width="1.4" opacity=".7">' + knurl + '</g></g>' +
    '<circle cx="52" cy="52" r="31" fill="none" stroke="url(#kRim)" stroke-width="1"/>' +
    '<circle cx="52" cy="52" r="24.5" fill="url(#kCap)"/>' +
    '<circle cx="52" cy="52" r="24.5" fill="none" stroke="url(#kRim)" stroke-width="1"/>' +
    '<ellipse cx="45" cy="42" rx="13" ry="7" fill="url(#kGloss)" transform="rotate(-30 45 42)"/>' +
    '<g transform="rotate(' + ang.toFixed(1) + ' 52 52)"><line x1="' + px0 + '" y1="' + py0 + '" x2="' + px1 + '" y2="' + py1 + '" stroke="#0a0c0e" stroke-width="4" stroke-linecap="round"/>' +
    '<line x1="' + px0 + '" y1="' + py0 + '" x2="' + px1 + '" y2="' + py1 + '" stroke="#f1f3f5" stroke-width="2" stroke-linecap="round"/></g>' +
    '</svg>';
}

// ------------------------------------------------------------------ the scope (b.html)
const sc = $('scope');
let scopeGeo = null, scopeDrag = null, scopeHover = null;
const DB_LO = -72, DB_HI = 0;
function sy(db, h) { return 8 + (h - 30) * (1 - (clamp(db, DB_LO, DB_HI) - DB_LO) / (DB_HI - DB_LO)); }
// Drawn the way dynamics plugins draw it (FabFilter Pro-C 2 and Pro-G, iZotope Neutron): every pixel column is the
// loudest moment of the time it covers, so a voice reads as fine spikes; what came in a pale area, what goes out a
// stronger one over it, no outlines; the gain reduction hanging from the top
function drawScope() {
  const [g, w, h] = fitCanvas(sc);
  const L = 36, R = w - 8, W = R - L, top = 8, bot = h - 22;
  scopeGeo = { h, L, R };
  const bg = g.createLinearGradient(0, top, 0, bot);
  bg.addColorStop(0, '#0d1013'); bg.addColorStop(1, '#090b0d');
  g.fillStyle = bg; g.fillRect(0, 0, w, h);
  g.font = '10px ' + MONO; g.textAlign = 'right'; g.textBaseline = 'middle';
  for (let d = 0; d >= -72; d -= 12) {
    const y = sy(d, h);
    g.fillStyle = 'rgba(255,255,255,.045)'; g.fillRect(L, Math.round(y), W, 1);
    g.fillStyle = css('--faint'); g.fillText(d === 0 ? '0' : '−' + -d, L - 6, y);
  }
  // the time axis; "now" ends at the right edge rather than past it
  for (let s = 0; s <= 10; s += 2) {
    const x = R - (s / 10) * W;
    g.fillStyle = 'rgba(255,255,255,.03)'; g.fillRect(Math.round(x), top, 1, bot - top);
    g.fillStyle = css('--faint'); g.textAlign = s === 0 ? 'right' : 'center'; g.font = s === 0 ? '10px ' + UIF : '10px ' + MONO;
    g.fillText(s === 0 ? t('now') : '−' + s + ' s', x, h - 9);
  }
  g.font = '10px ' + MONO;
  // one value per pixel column: the loudest in, the loudest out, the most turned down
  const cols = Math.max(1, Math.floor(W));
  const ci = new Float32Array(cols), co = new Float32Array(cols), cg = new Float32Array(cols);
  for (let c = 0; c < cols; c++) {
    const a = Math.floor((c / cols) * HN), b = Math.max(a + 1, Math.floor(((c + 1) / cols) * HN));
    let mi = -200, mo = -200, mg = 0;
    for (let k = a; k < b; k++) {
      const j = (H.head + k) % HN;
      if (H.inp[j] > mi) mi = H.inp[j];
      if (H.out[j] > mo) mo = H.out[j];
      if (H.gr[j] < mg) mg = H.gr[j];
    }
    ci[c] = mi; co[c] = mo; cg[c] = mg;
  }
  const area = (vals, fill) => {
    g.beginPath(); g.moveTo(L, bot);
    for (let c = 0; c < cols; c++) { const y = sy(vals[c], h); g.lineTo(L + c, y); g.lineTo(L + c + 1, y); }
    g.lineTo(L + cols, bot); g.closePath(); g.fillStyle = fill; g.fill();
  };
  // in: pale; out: the accent, stronger near its top
  area(ci, 'rgba(150,158,170,.30)');
  const og = g.createLinearGradient(0, top, 0, bot);
  og.addColorStop(0, 'rgba(79,214,180,.85)'); og.addColorStop(0.55, 'rgba(79,214,180,.45)'); og.addColorStop(1, 'rgba(79,214,180,.12)');
  area(co, og);
  // what came in, its edge over the output too, so both read
  g.beginPath();
  for (let c = 0; c < cols; c++) { const y = sy(ci[c], h); c ? g.lineTo(L + c + 0.5, y) : g.moveTo(L, y); }
  g.strokeStyle = 'rgba(200,206,214,.45)'; g.lineWidth = 1; g.stroke();
  // turned down: from the top, 1 px a dB at 3x, a line and a faint fill
  const gy = (d) => top + clamp(-d, 0, 24) * 3;
  g.beginPath(); g.moveTo(L, top);
  for (let c = 0; c < cols; c++) g.lineTo(L + c + 0.5, gy(cg[c]));
  g.lineTo(L + cols, top); g.closePath(); g.fillStyle = 'rgba(242,165,65,.16)'; g.fill();
  g.beginPath();
  for (let c = 0; c < cols; c++) { const y = gy(cg[c]); c ? g.lineTo(L + c + 0.5, y) : g.moveTo(L, y); }
  g.strokeStyle = css('--comp'); g.lineWidth = 1.2; g.stroke();
  // the room's floor, once measured
  if (M.floor > -110) {
    g.setLineDash([3, 3]); g.strokeStyle = 'rgba(139,146,157,.8)'; g.lineWidth = 1;
    g.beginPath(); g.moveTo(L, Math.round(sy(M.floor, h)) + 0.5); g.lineTo(R, Math.round(sy(M.floor, h)) + 0.5); g.stroke(); g.setLineDash([]);
    g.textAlign = 'left'; g.fillStyle = '#8b929d'; g.font = '10px ' + UIF; g.fillText(t('floor_short') + ' ' + fmtDb(M.floor), L + 6, sy(M.floor, h) + 9); g.font = '10px ' + MONO;
  }
  // a line's tag: its name in the interface face, its value in the mono one, measured in the faces they are drawn in
  const line = (db, color, name, key) => {
    const y = Math.round(sy(db, h)), active = scopeDrag === key || scopeHover === key;
    g.fillStyle = color; g.globalAlpha = active ? 1 : 0.8; g.fillRect(L, y - (active ? 1 : 0), W, active ? 2 : 1); g.globalAlpha = 1;
    const val = fmtDb(db) + ' dB', fN = '600 11px ' + UIF, fV = '600 11px ' + MONO;
    g.font = fN; const wN = g.measureText(name).width;
    g.font = fV; const wV = g.measureText(val).width;
    const PAD = 10, GAP = 7, tw = PAD + wN + GAP + wV + PAD, bx = R - tw - 6;
    g.fillStyle = color; g.beginPath(); g.roundRect(bx, y - 10, tw, 20, 2); g.fill();
    g.fillStyle = '#0b0d10'; g.textAlign = 'left'; g.textBaseline = 'middle';
    g.font = fN; g.fillText(name, bx + PAD, y + 0.5);
    g.font = fV; g.fillText(val, bx + PAD + wN + GAP, y + 0.5);
    g.font = '10px ' + MONO;
  };
  if (P().on.exp && P().knobs.gate > -80) line(P().knobs.gate, css('--gate'), t('gate_line'), 'gate');
  if (P().on.comp) line(P().comp.thr, css('--comp'), t('comp_line'), 'comp');
  if (S.bypass) { g.fillStyle = 'rgba(15,17,20,.55)'; g.fillRect(L, top, W, bot - top); g.fillStyle = css('--comp'); g.textAlign = 'center'; g.font = '600 14px ' + UIF; g.fillText(t('bypass_scope'), L + W / 2, h / 2); g.font = '10px ' + MONO; }
}

// ------------------------------------------------------------------ the EQ as a curve: Low Cut moves left and right, the
// three others up and down. The curve is the engine's own response (crates/dsp/src/eq.rs: Butterworth high pass, shelves at
// 200 Hz and 10 kHz with Q 0.707, a 3 kHz bell with Q 1; Simper SVFs, which are the RBJ biquads at 48 kHz). A shelf's
// point sits on the curve at its corner, where it has half its gain, as in most EQs.
const EQF = { low: 200, pres: 3000, air: 10000 }, EQQ = { hp: Math.SQRT1_2, low: Math.SQRT1_2, pres: 1, air: Math.SQRT1_2 };
const FS = 48000;
function biquadDb(kind, f0, q, gain, f) {
  const A = Math.pow(10, gain / 40), w0 = 2 * Math.PI * f0 / FS, cw = Math.cos(w0), al = Math.sin(w0) / (2 * q);
  let b, a;
  if (kind === 'hp') { b = [(1 + cw) / 2, -(1 + cw), (1 + cw) / 2]; a = [1 + al, -2 * cw, 1 - al]; }
  else if (kind === 'bell') { b = [1 + al * A, -2 * cw, 1 - al * A]; a = [1 + al / A, -2 * cw, 1 - al / A]; }
  else {
    const s = kind === 'ls' ? 1 : -1, r = 2 * Math.sqrt(A) * al;
    b = [A * ((A + 1) - s * (A - 1) * cw + r), s * 2 * A * ((A - 1) - s * (A + 1) * cw), A * ((A + 1) - s * (A - 1) * cw - r)];
    a = [(A + 1) + s * (A - 1) * cw + r, -s * 2 * ((A - 1) + s * (A + 1) * cw), (A + 1) + s * (A - 1) * cw - r];
  }
  const w = 2 * Math.PI * f / FS, c1 = Math.cos(w), s1 = Math.sin(w), c2 = Math.cos(2 * w), s2 = Math.sin(2 * w);
  const mag = (k) => Math.hypot(k[0] + k[1] * c1 + k[2] * c2, k[1] * s1 + k[2] * s2);
  return 20 * Math.log10(mag(b) / mag(a));
}
function eqResponse(f, e) {
  return (e.hp > 0 ? biquadDb('hp', e.hp, EQQ.hp, 0, f) : 0) + biquadDb('ls', EQF.low, EQQ.low, e.low, f) +
    biquadDb('bell', EQF.pres, EQQ.pres, e.pres, f) + biquadDb('hs', EQF.air, EQQ.air, e.air, f);
}
// shelves show (and are dragged by) half their gain: the point stays on the curve
const EQ_HALF = { low: true, air: true };
function eqPoints() {
  const e = P().eq;
  return [{ id: 'hp', name: 'Low Cut', f: e.hp, g: 0 }, { id: 'low', name: 'Low', f: EQF.low, g: e.low / 2 }, { id: 'pres', name: 'Presence', f: EQF.pres, g: e.pres }, { id: 'air', name: 'Air', f: EQF.air, g: e.air / 2 }];
}
function drawEq(c) {
  const [g, w, h] = fitCanvas(c);
  const fx = (f) => 30 + ((Math.log10(f) - Math.log10(20)) / 3) * (w - 40);
  const dy = (d) => h / 2 - (d / 12) * (h / 2 - 14);
  g.fillStyle = css('--inset'); g.fillRect(0, 0, w, h);
  g.font = '10px ' + MONO; g.textAlign = 'center';
  for (const f of [50, 100, 200, 500, 1000, 2000, 5000, 10000]) {
    g.fillStyle = 'rgba(255,255,255,.04)'; g.fillRect(Math.round(fx(f)), 0, 1, h);
    g.fillStyle = css('--faint'); g.fillText(f >= 1000 ? f / 1000 + 'k' : f, fx(f), h - 4);
  }
  g.textAlign = 'left';
  for (const d of [12, 6, 0, -6, -12]) { g.fillStyle = d === 0 ? 'rgba(255,255,255,.09)' : 'rgba(255,255,255,.04)'; g.fillRect(30, Math.round(dy(d)), w - 40, 1); g.fillStyle = css('--faint'); g.fillText((d > 0 ? '+' : '') + d, 4, dy(d) + 3); }
  const e = P().eq;
  g.beginPath();
  for (let i = 0; i <= 300; i++) { const f = 20 * Math.pow(1000, i / 300), y = dy(clamp(eqResponse(f, e), -14, 14)); i ? g.lineTo(fx(f), y) : g.moveTo(fx(f), y); }
  g.strokeStyle = css('--accent'); g.lineWidth = 2; g.stroke();
  g.lineTo(fx(20000), h); g.lineTo(fx(20), h); g.fillStyle = 'rgba(79,214,180,.08)'; g.fill();
  for (const p of eqPoints()) {
    const x = fx(p.f), y = dy(p.g);
    g.beginPath(); g.arc(x, y, 7, 0, 7); g.fillStyle = css('--panel2'); g.fill(); g.strokeStyle = css('--accent'); g.lineWidth = 2; g.stroke();
    g.fillStyle = css('--muted'); g.font = '11px ' + UIF; g.textAlign = 'center'; g.fillText(p.name, x, y - 12);
  }
  return { fx, dy, w, h };
}
function eqDrag(c) {
  let geo = drawEq(c), hold = null;
  const send = throttled((ch) => post({ preset: { eq: ch } }));
  const pos = (e) => { const r = c.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; };
  c.addEventListener('pointerdown', (e) => {
    const [x, y] = pos(e);
    hold = eqPoints().find((p) => Math.hypot(geo.fx(p.f) - x, geo.dy(p.g) - y) < 14) || null;
    if (hold) { c.setPointerCapture(e.pointerId); holding.add('eq'); }
  });
  c.addEventListener('pointermove', (e) => {
    if (!hold) return;
    const [x, y] = pos(e);
    if (hold.id === 'hp') {
      const f = Math.pow(10, Math.log10(20) + ((x - 30) / (geo.w - 40)) * 3);
      P().eq.hp = clamp(Math.round(f / 5) * 5, 40, 200);
    } else P().eq[hold.id] = clamp(Math.round(((geo.h / 2 - y) / (geo.h / 2 - 14)) * 12 * (EQ_HALF[hold.id] ? 2 : 1) * 2) / 2, -6, 6);
    geo = drawEq(c);
    send.move({ [hold.id]: P().eq[hold.id] });
  });
  c.addEventListener('pointerup', () => { if (hold) send.end({ [hold.id]: P().eq[hold.id] }); hold = null; holding.delete('eq'); });
  new ResizeObserver(() => (geo = drawEq(c))).observe(c);
}

// ------------------------------------------------------------------ building the page (once the state is in)
function build() {
  applyI18n();
  // header: A / B and the settings
  const sw = $('layoutSw');
  for (const b of sw.children) {
    b.setAttribute('aria-pressed', b.dataset.v === LAYOUT);
    b.onclick = () => { if (b.dataset.v !== LAYOUT) post({ ui: { layout: b.dataset.v } }, () => location.replace('/')); };
  }
  buildSettings();
  $('bypass').onclick = () => post({ bypass: !S.bypass });
  let spaceHeld = false;
  addEventListener('keydown', (e) => { if (e.code === 'Space' && !e.repeat && !/INPUT|SELECT|BUTTON|TEXTAREA/.test(document.activeElement.tagName) && !document.activeElement.classList.contains('knob')) { e.preventDefault(); spaceHeld = true; post({ bypass: true }); } });
  addEventListener('keyup', (e) => { if (e.code === 'Space' && spaceHeld) { spaceHeld = false; post({ bypass: false }); } });
  $('auto').onclick = autoStart;
  if (LAYOUT === 'a') buildKnobs(); else buildSliders();
  buildModes();
  buildChain();
  $('side').innerHTML = routingHtml();
  wireRouting();
}

// modes: a.html's screen (‹ ›), b.html's three buttons
function buildModes() {
  if (LAYOUT === 'a') {
    const step = (d) => { const i = S.builtins.findIndex((b) => b.name === P().name); post({ preset_name: S.builtins[(i + d + S.builtins.length) % S.builtins.length].name }); };
    $('prev').onclick = () => step(-1);
    $('next').onclick = () => step(1);
    $('dots').innerHTML = S.builtins.map(() => '<i></i>').join('');
    $('dots').onclick = (e) => { const i = [...$('dots').children].indexOf(e.target); if (i >= 0) post({ preset_name: S.builtins[i].name }); };
  } else {
    $('modes').innerHTML = S.builtins.map((b) => '<button data-n="' + b.name + '"><span></span><small></small></button>').join('');
    $('modes').onclick = (e) => { const b = e.target.closest('button'); if (b) post({ preset_name: b.dataset.n }); };
  }
}
function modesUI() {
  const cur = S.builtins.find((b) => b.name === P().name) || S.builtins[0];
  if (LAYOUT === 'a') {
    $('micon').textContent = MODE_ICON[cur.name] || '◉';
    $('mname').innerHTML = modeName(cur) + (cur.edited ? '<span class="edited">' + t('edited') + '</span>' : '');
    $('about').textContent = modeAbout(cur);
    [...$('dots').children].forEach((d, i) => d.classList.toggle('on', S.builtins[i].name === cur.name));
  } else {
    for (const b of $('modes').children) {
      const m = S.builtins.find((x) => x.name === b.dataset.n);
      b.setAttribute('aria-pressed', m.name === cur.name);
      b.querySelector('span').textContent = (MODE_ICON[m.name] || '') + ' ' + modeName(m);
      b.querySelector('small').textContent = m.edited ? t('edited') : '';
    }
  }
}

// a.html: knobs (drag up and down, Shift for fine, the wheel, arrow keys)
function buildKnobs() {
  const box = $('knobs');
  box.innerHTML = KNOBS.map((k) => '<div class="knob" data-k="' + k.id + '" tabindex="0" role="slider"><div class="sv"></div><div class="kname"></div><div class="kval val"></div></div>').join('');
  for (const el of box.children) {
    const k = KNOBS.find((x) => x.id === el.dataset.k);
    let y0 = null, v0 = 0;
    const set = (v, end) => {
      const x = clamp(Math.round(v), k.min, k.max()), s = setKnob(k.id, x);
      end ? s.end(x) : s.move(x);
      knobsUI();
    };
    el.addEventListener('pointerdown', (e) => { y0 = e.clientY; v0 = P().knobs[k.id]; el.setPointerCapture(e.pointerId); holding.add('knob:' + k.id); });
    el.addEventListener('pointermove', (e) => { if (y0 !== null) set(v0 + ((y0 - e.clientY) * (k.max() - k.min)) / (e.shiftKey ? 600 : 160)); });
    el.addEventListener('pointerup', (e) => { if (y0 !== null) set(v0 + ((y0 - e.clientY) * (k.max() - k.min)) / (e.shiftKey ? 600 : 160), true); y0 = null; holding.delete('knob:' + k.id); });
    el.addEventListener('wheel', (e) => { e.preventDefault(); set(P().knobs[k.id] - Math.sign(e.deltaY) * (k.max() - k.min) / 50, true); }, { passive: false });
    el.addEventListener('keydown', (e) => { const d = { ArrowUp: 1, ArrowRight: 1, ArrowDown: -1, ArrowLeft: -1 }[e.key]; if (d) { e.preventDefault(); set(P().knobs[k.id] + d * (k.max() - k.min) / 50, true); } });
  }
}
function knobsUI() {
  for (const el of $('knobs').children) {
    const k = KNOBS.find((x) => x.id === el.dataset.k), v = P().knobs[k.id];
    el.querySelector('.sv').innerHTML = knobSvg(v, k);
    el.querySelector('.kname').textContent = k.name();
    el.querySelector('.kval').textContent = k.word(v) + (k.id === 'gate' && M.floor > -110 ? ' · ' + t('floor_tick') + ' ' + fmtDb(M.floor).replace('.0', '') : '');
    el.classList.toggle('dim', (k.id === 'denoise' || k.id === 'space') && v === 0);
    el.setAttribute('aria-label', k.name());
    el.setAttribute('aria-valuenow', v);
    if (k.id === 'denoise') el.title = sing() ? t('sing_denoise') : '';
  }
}
// b.html: the same as sliders
function buildSliders() {
  const box = $('sliders');
  box.innerHTML = KNOBS.map((k) => '<label class="lvl' + (k.id === 'gate' ? ' gate' : '') + '"><span></span><input type="range" data-k="' + k.id + '" step="1"><output></output></label>').join('');
  for (const inp of box.querySelectorAll('input')) {
    const id = inp.dataset.k;
    inp.addEventListener('input', () => { holding.add('sl:' + id); setKnob(id, +inp.value).move(+inp.value); slidersUI(); });
    inp.addEventListener('change', () => { setKnob(id, +inp.value).end(+inp.value); holding.delete('sl:' + id); });
  }
}
function slidersUI() {
  for (const inp of $('sliders').querySelectorAll('input')) {
    const k = KNOBS.find((x) => x.id === inp.dataset.k);
    inp.min = k.min; inp.max = k.max();
    if (!holding.has('sl:' + k.id)) inp.value = P().knobs[k.id];
    inp.previousElementSibling.textContent = k.name();
    inp.nextElementSibling.textContent = k.word(P().knobs[k.id]);
  }
}

// the chain: a module per stage; one picked shows its details
let picked = null;
function buildChain() {
  const box = $('chain');
  box.innerHTML = STAGES.map((s) => '<button class="mod" data-s="' + s.id + '"><div class="name">' + s.name + '</div><div class="meta"><span class="sub"></span><span class="led"></span></div><div class="bar"><i></i></div><span class="arrow"></span></button>').join('');
  box.onclick = (e) => {
    const m = e.target.closest('.mod');
    if (!m) return;
    picked = picked === m.dataset.s ? null : m.dataset.s;
    for (const x of box.children) x.setAttribute('aria-selected', x.dataset.s === picked);
    buildDetail();
  };
}
function chainUI() {
  for (const m of $('chain').children) {
    const id = m.dataset.s;
    m.classList.toggle('on', !!P().on[id] && !S.bypass);
    m.querySelector('.sub').textContent = msText(stageLatency(id));
  }
}
function chainTick() {
  for (const m of $('chain').children) {
    const id = m.dataset.s, v = id === 'exp' ? M.grGate : id === 'comp' ? M.grComp : id === 'lim' ? M.grLim : 0;
    m.querySelector('.bar i').style.width = clamp(v / 12, 0, 1) * 100 + '%';
  }
}
function buildDetail() {
  const d = $('detail');
  if (!picked) { d.hidden = LAYOUT === 'a'; d.innerHTML = LAYOUT === 'b' ? '<p class="note">' + t('pick_stage') + '</p>' : ''; return; }
  const s = STAGES.find((x) => x.id === picked);
  d.hidden = false;
  d.innerHTML = '<div class="dh"><b>' + s.name + '</b><span class="note num" style="margin:0" data-lat></span><span class="spacer"></span>' +
    '<label>' + t('in_phones') + '<button class="sw" data-mon role="switch"></button></label>' +
    '<label>' + t('on') + '<button class="sw" data-on role="switch"></button></label></div>' +
    (LAYOUT === 'b' && (s.id === 'exp' || s.id === 'comp') ? '<p class="note" style="margin-top:0">' + t('drag_too').replace(/^；|^; /, '') + '</p>' : '') +
    (s.id === 'eq' ? '<canvas id="eqc"></canvas><p class="note">' + t('eq_note') + '</p>' : '') +
    (s.id === 'verb' ? '<div class="seg" id="vstyles" style="grid-auto-flow:row;grid-template-columns:repeat(4,1fr)">' + VERB_STYLES.map(([v, n]) => '<button data-v="' + v + '">' + n + '</button>').join('') + '</div><p class="note"><span id="vcustom"></span>' + t('verb_note') + '</p>' : '') +
    s.params.map(([id, name, lo, hi, st, u, only]) => '<label class="lvl" data-p="' + id + '"' + (only ? ' data-only="' + only + '"' : '') + '><span>' + name + '</span><input type="range" min="' + lo + '" max="' + hi + '" step="' + st + '"><output></output></label>').join('');
  d.querySelector('[data-on]').onclick = () => post({ preset: { on: { [s.id]: !P().on[s.id] } } });
  d.querySelector('[data-mon]').onclick = () => post({ preset: { monitor: { [s.id]: !P().monitor[s.id] } } });
  for (const row of d.querySelectorAll('.lvl[data-p]')) {
    const id = row.dataset.p, inp = row.querySelector('input'), q = s.params.find((x) => x[0] === id);
    const send = throttled((v) => post({ preset: { [s.id]: { [id]: v } } }));
    inp.addEventListener('input', () => { holding.add('p:' + id); P()[s.id][id] = +inp.value; row.querySelector('output').textContent = fmtP(inp.value, q[4]) + (q[5] ? ' ' + q[5] : ''); send.move(+inp.value); });
    inp.addEventListener('change', () => { send.end(+inp.value); holding.delete('p:' + id); });
  }
  if (s.id === 'eq') eqDrag($('eqc'));
  if (s.id === 'verb') for (const b of $('vstyles').children) b.onclick = () => post({ preset: { verb: { style: b.dataset.v } } });
  detailUI();
}
function detailUI() {
  if (!picked) return;
  const d = $('detail'), s = STAGES.find((x) => x.id === picked);
  d.querySelector('[data-on]').setAttribute('aria-checked', !!P().on[s.id]);
  d.querySelector('[data-mon]').setAttribute('aria-checked', !!P().monitor[s.id]);
  d.querySelector('[data-lat]').textContent = msText(stageLatency(s.id));
  for (const row of d.querySelectorAll('.lvl[data-p]')) {
    const id = row.dataset.p, inp = row.querySelector('input'), q = s.params.find((x) => x[0] === id), v = P()[s.id][id];
    row.hidden = row.dataset.only === 'sing' && !sing();
    if (s.id === 'verb' && id === 'mix') row.querySelector('span').textContent = sing() ? 'Mix (with backing)' : 'Mix';
    if (!holding.has('p:' + id)) inp.value = v;
    row.querySelector('output').textContent = fmtP(v, q[4]) + (q[5] ? ' ' + q[5] : '');
  }
  if (s.id === 'eq' && !holding.has('eq')) drawEq($('eqc'));
  if (s.id === 'verb') {
    for (const b of $('vstyles').children) b.setAttribute('aria-pressed', b.dataset.v === P().verb.style);
    $('vcustom').textContent = P().verb.style ? '' : t('custom') + '。';
  }
}

// the computer's sound, the monitor, the devices, start with the computer
function routingHtml() {
  return '<section class="card"><h2>' + t('pc') + '</h2>' +
    '<div class="row" style="grid-template-columns:1fr auto"><span>' + t('lb_on') + '</span><button class="sw" id="lbOn" role="switch" aria-label="' + t('lb_on') + '"></button></div>' +
    '<label class="lvl" id="lbRow"><span>' + t('lb_db') + '</span><input type="range" id="lbDb" min="-30" max="6" step="0.5"><output></output></label>' +
    '<label class="lvl" id="pcRow"><span>' + t('pc_db') + '</span><input type="range" id="pcDb" min="-30" max="6" step="0.5"><output></output></label>' +
    '<p class="warnline note" id="clipLine"></p><p class="note" id="pcNote"></p></section>' +
    '<section class="card"><h2>' + t('mon') + '</h2>' +
    '<div class="seg" id="mon"><button data-v="processed">' + t('mon_proc') + '</button><button data-v="direct">' + t('mon_direct') + '</button><button data-v="off">' + t('mon_off') + '</button></div>' +
    '<label class="lvl" id="monRow"><span>' + t('mon_db') + '</span><input type="range" id="monDb" min="-30" max="6" step="0.5"><output></output></label>' +
    '<p class="note" id="monNote"></p></section>' +
    '<section class="card"><h2>' + t('devices') + '</h2>' +
    '<label class="row"><span>' + t('mic') + '</span><select id="devIn"></select></label>' +
    '<label class="row"><span>' + t('phones') + '</span><select id="devOut"></select></label>' +
    '<div class="row"><span>' + t('vmic') + '</span><b id="vmName"></b></div>' +
    '<p class="note" id="devWarn" style="color:var(--comp)"></p><p class="note" id="vmNote"></p>' +
    '<div id="appBox" hidden><div class="row" style="grid-template-columns:1fr auto;margin-top:12px"><span>' + t('autostart') + '</span><button class="sw" id="autoOn" role="switch" aria-label="' + t('autostart') + '"></button></div>' +
    '<p class="note">' + t('tray_note') + '</p></div></section>';
}
function level(id, key) {
  const inp = $(id), send = throttled((v) => post({ [key]: v }));
  inp.addEventListener('input', () => { holding.add(id); S[key] = +inp.value; inp.nextElementSibling.textContent = fmtDb(+inp.value) + ' dB'; send.move(+inp.value); });
  inp.addEventListener('change', () => { send.end(+inp.value); holding.delete(id); });
}
function wireRouting() {
  $('lbOn').onclick = () => post({ loopback: !S.loopback });
  level('lbDb', 'loopback_db'); level('pcDb', 'pc_db'); level('monDb', 'monitor_db');
  for (const b of $('mon').children) b.onclick = () => post({ monitor: b.dataset.v });
  $('devIn').onchange = (e) => post({ mic: e.target.value });
  $('devOut').onchange = (e) => post({ out: e.target.value });
  $('autoOn').onclick = () => post({ autostart: !S.autostart });
}
function routingUI() {
  $('lbOn').setAttribute('aria-checked', S.loopback);
  $('lbRow').classList.toggle('dim', !S.loopback);
  for (const [id, key] of [['lbDb', 'loopback_db'], ['pcDb', 'pc_db'], ['monDb', 'monitor_db']]) {
    if (!holding.has(id)) $(id).value = S[key];
    $(id).nextElementSibling.textContent = fmtDb(S[key]) + ' dB';
  }
  // Windows plays the computer's sound to the headphones itself: no volume for it here
  $('pcRow').hidden = win();
  $('pcNote').textContent = win() ? t('pc_note_win') : t('pc_note_linux', { sink: S.sink_desc || 'stream-mic 電腦聲音' });
  for (const b of $('mon').children) b.setAttribute('aria-pressed', b.dataset.v === S.monitor);
  $('monRow').hidden = S.monitor !== 'processed';
  $('monNote').textContent = t('mon_note_' + S.monitor);
  $('vmName').textContent = win() ? 'CABLE Output (VB-CABLE)' : 'stream-mic';
  $('vmNote').textContent = win() ? t('obs_note_win') : t('obs_note_linux');
  $('appBox').hidden = !(S.app && S.autostart_ok);
  $('autoOn').setAttribute('aria-checked', !!S.autostart);
}
function devicesUI() {
  if (!D || !$('devIn')) return;
  const opts = (list, cur) => list.map((x) => '<option value="' + x.name.replace(/"/g, '&quot;') + '"' + (x.name === cur ? ' selected' : '') + '>' + (x.desc || x.name) + '</option>').join('');
  // the mic in use is always in the list (a sink's monitor used as the mic is not a source)
  const mics = (D.mics || []).slice();
  if (D.mic && !mics.some((x) => x.name === D.mic.name)) mics.unshift(D.mic);
  $('devIn').innerHTML = opts(mics, D.mic && D.mic.name);
  $('devOut').innerHTML = opts(D.outs || [], D.out && D.out.name);
  const none = [!D.mic && t('dev_mic'), !D.out && t('dev_out_any')].filter(Boolean);
  const miss = [D.mic_missing && D.mic && t('dev_mic'), D.out_missing && D.out && t('dev_out')].filter(Boolean);
  $('devWarn').textContent = [
    none.length ? t('dev_none', { what: none.join(t('dev_and')) }) : '',
    miss.length ? t('dev_missing', { what: miss.join(t('dev_and')) }) : '',
    win() && !D.cable ? t('no_cable') : '',
    win() && D.loopback_ok === false ? t('no_loopback') : '',
  ].filter(Boolean).join(' ');
}

// everything that follows the state
function render() {
  if (!S) return;
  modesUI();
  if (LAYOUT === 'a') knobsUI(); else slidersUI();
  chainUI();
  detailUI();
  routingUI();
  devicesUI();
  const bp = $('bypass');
  bp.setAttribute('aria-pressed', S.bypass);
  bp.textContent = S.bypass ? (LAYOUT === 'a' ? t('bypass_on') : t('bypass_on_short')) : t('bypass');
}

// ------------------------------------------------------------------ the header, the warnings, the counters
let lastBusy = null, clipAt = -1e9, sendClip = null;
function stats() {
  if (!online) return;
  fetch('/api/stats').then((r) => r.json()).then((s) => {
    const now = performance.now();
    if (lastBusy) $('cpuChip').innerHTML = 'CPU <span class="num">' + Math.max(0, (s.busy_us - lastBusy.us) / 1000 / (now - lastBusy.t) * 100).toFixed(1) + ' %</span>';
    lastBusy = { us: s.busy_us, t: now };
    // the send guard bent the voice and the computer's sound together: say so for a few seconds
    if (sendClip !== null && s.send_clip > sendClip) clipAt = now;
    sendClip = s.send_clip;
    ST = s;
    // what the voice takes to go out: a cycle in and one out, and the chain's own
    const buf = 2 * (s.quantum || 128) / 48, send = buf + (s.send_latency_ms || 0);
    $('latChip').innerHTML = t('send_lat') + ' <span class="num">' + send.toFixed(1) + ' ms</span>';
    M.loop = M.loop || !!s.loop_mute;
    chainUI();
    detailUI();
  }).catch(() => {});
}
function bannerUI() {
  const b = $('banner');
  const msg = !online ? ['bad', t('offline_msg')] : M.loop ? ['bad', t('loop')] : performance.now() - clipAt < 5000 ? ['warn', t('clip')] : null;
  b.hidden = !msg;
  if (msg) { b.className = 'banner ' + msg[0]; b.textContent = msg[1]; }
  const st = $('state');
  st.textContent = !online ? t('offline') : S && S.bypass ? t('bypassed') : t('running');
  st.className = 'chip ' + (!online || (S && S.bypass) ? 'off' : 'ok');
  $('lufsChip').innerHTML = t('loudness') + ' <span class="num">' + (M.lufs > -60 ? fmtDb(M.lufs) : '−∞') + ' LUFS</span>';
}

// ------------------------------------------------------------------ auto setup (the engine measures; /api/autoset)
let autoTimer = null;
function autoPost(c) {
  return fetch('/api/autoset', { method: 'POST', body: JSON.stringify(c) }).then((r) => (r.ok ? r.json() : r.text().then((x) => { throw new Error(x); })));
}
function autoStart() { autoPost({ start: true }).then(autoUI).catch((e) => autoUI({ phase: 'error', error: e.message })); }
// on load only a run in progress shows: the last one's result (done, cancelled) does not come back on every reload
function autoPoll(first) { fetch('/api/autoset').then((r) => r.json()).then((a) => { if (!first || ['quiet', 'talk', 'working'].includes(a.phase)) autoUI(a); }).catch(() => {}); }
const sgn = (v) => (v > 0 ? '+' : v < 0 ? '−' : '') + Math.abs(v).toFixed(1);
function autoUI(a) {
  let box = $('autoBox');
  if (a.phase === 'idle') { if (box) box.remove(); return; }
  if (!box) {
    box = document.createElement('div');
    box.id = 'autoBox'; box.className = 'overlay';
    box.innerHTML = '<section class="card dialog" role="dialog" aria-modal="true"><h2>' + t('au_title') + '</h2><div class="step" id="auStep"></div>' +
      '<div class="progress" id="auProg"><i></i></div><p class="note" id="auText"></p><ul class="warns" id="auWarn"></ul>' +
      '<div class="actions"><button class="btn" id="auAgain">' + t('au_again') + '</button><button class="btn" id="auUndo">' + t('au_undo') + '</button>' +
      '<button class="btn" id="auCancel">' + t('au_cancel') + '</button><button class="btn primary" id="auClose">' + t('au_close') + '</button></div></section>';
    document.body.append(box);
    $('auAgain').onclick = autoStart;
    $('auUndo').onclick = () => autoPost({ undo: true }).then(autoUI).catch((e) => autoUI({ phase: 'error', error: e.message }));
    $('auCancel').onclick = () => autoPost({ cancel: true }).then(autoUI).catch(() => {});
    $('auClose').onclick = () => box.remove();
  }
  const busy = ['quiet', 'talk', 'working'].includes(a.phase);
  $('auCancel').hidden = !(a.phase === 'quiet' || a.phase === 'talk');
  $('auAgain').hidden = busy;
  $('auClose').hidden = busy;
  $('auUndo').hidden = !(a.phase === 'done' && a.applied);
  $('auProg').hidden = !busy;
  const prog = (whole) => { $('auProg').firstChild.style.width = (100 * (1 - (a.left_s || 0) / whole)) + '%'; };
  const w = [];
  if (a.phase === 'quiet') { $('auStep').textContent = t('au_quiet', { s: Math.ceil(a.left_s || 0) }); $('auText').textContent = t('au_quiet_t'); prog(3); }
  else if (a.phase === 'talk') { $('auStep').textContent = t('au_talk', { s: Math.ceil(a.left_s || 0) }); $('auText').innerHTML = t('au_talk_t') + '<span class="say">' + t('au_say') + '</span>'; prog(10); }
  else if (a.phase === 'working') { $('auStep').textContent = t('au_working'); $('auText').textContent = ''; $('auProg').firstChild.style.width = '100%'; }
  else if (a.phase === 'done') {
    $('auStep').textContent = a.applied ? t('au_done') : t('au_not');
    $('auText').textContent = a.applied ? t('au_result', { gate: a.gate, floor: a.floor_db.toFixed(0), thr: a.comp_thr, gr: Math.abs(a.comp_gr_db).toFixed(1), mk: sgn(a.makeup), lufs: a.lufs.toFixed(1), target: a.target }) : '';
    for (const x of a.warnings || []) {
      if (x.kind === 'novoice') w.push(t('au_novoice'));
      if (x.kind === 'noisy') w.push(t('au_noisy', { d: x.db.toFixed(0) }));
      if (x.kind === 'quiet') w.push(t('au_low', { d: sgn(x.db) }));
      if (x.kind === 'loud') w.push(t('au_loud', { d: x.db.toFixed(1) }));
    }
    if (a.hears_pc) w.push(t('au_hears_pc'));
  } else if (a.phase === 'error') { $('auStep').textContent = t('au_error'); $('auText').textContent = a.error || ''; }
  else if (a.phase === 'cancelled') { $('auStep').textContent = t('au_cancelled'); $('auText').textContent = t('au_cancelled_t'); }
  else if (a.phase === 'undone') { $('auStep').textContent = t('au_undone'); $('auText').textContent = t('au_undone_t'); }
  $('auWarn').innerHTML = w.map((x) => '<li>' + x + '</li>').join('');
  clearTimeout(autoTimer);
  if (busy) autoTimer = setTimeout(() => autoPoll(), 200);
  else if (a.phase === 'done' || a.phase === 'undone') fetch('/api/state').then((r) => r.json()).then((st) => { S = st; render(); }).catch(() => {});
}

// ------------------------------------------------------------------ the settings: layout, language
function buildSettings() {
  const gear = $('gear');
  const pop = document.createElement('div');
  pop.className = 'pop card';
  pop.hidden = true;
  const seg = (name, items) => '<div class="prow"><span>' + t(name) + '</span><div class="seg">' + items.map(([v, label, on, dis]) =>
    '<button data-v="' + v + '" aria-pressed="' + on + '"' + (dis ? ' disabled' : '') + '>' + label + '</button>').join('') + '</div></div>';
  pop.innerHTML = '<h2>' + t('settings') + '</h2>' +
    seg('layout', [['a', 'A ' + t('layout_a'), LAYOUT === 'a'], ['b', 'B ' + t('layout_b'), LAYOUT === 'b']]) +
    seg('language', [['zh', '繁中', LANG === 'zh'], ['en', 'English', LANG === 'en']]) +
    '<p class="note">' + t('layout_note') + '</p>';
  gear.after(pop);
  gear.onclick = (e) => { e.stopPropagation(); pop.hidden = !pop.hidden; };
  document.addEventListener('click', (e) => { if (!pop.contains(e.target) && e.target !== gear) pop.hidden = true; });
  pop.addEventListener('click', (e) => {
    const b = e.target.closest('button');
    if (!b || b.disabled) return;
    const v = b.dataset.v;
    if (v === 'a' || v === 'b') { if (v !== LAYOUT) post({ ui: { layout: v } }, () => location.replace('/')); }
    else if (v === 'zh' || v === 'en') { if (v !== LANG) post({ ui: { lang: v } }, () => location.reload()); }
  });
}

// ------------------------------------------------------------------ each frame
let balN = 0;
function tick() {
  if (S) {
    if ($('meters')) drawMeters($('meters'));
    if (sc) drawScope();
    if (++balN % 12 === 0) {
      $('bal').innerHTML = balanceHtml();
      bannerUI();
      if (LAYOUT === 'a') knobsUI();
    }
    chainTick();
  }
  requestAnimationFrame(tick);
}

// ------------------------------------------------------------------ b.html: dragging the thresholds on the scope
function nearLine(y) {
  if (!scopeGeo) return null;
  const c = [['gate', P().knobs.gate, P().on.exp && P().knobs.gate > -80], ['comp', P().comp.thr, P().on.comp]].filter((x) => x[2]).map(([k, db]) => [k, Math.abs(sy(db, scopeGeo.h) - y)]).sort((a, b) => a[1] - b[1]);
  return c.length && c[0][1] < 10 ? c[0][0] : null;
}
if (sc) {
  const toDb = (y) => DB_LO + (1 - (y - 8) / (scopeGeo.h - 30)) * (DB_HI - DB_LO);
  const sendComp = throttled((v) => post({ preset: { comp: { thr: v } } }));
  sc.addEventListener('pointerdown', (e) => { const y = e.clientY - sc.getBoundingClientRect().top; scopeDrag = nearLine(y); if (scopeDrag) { sc.setPointerCapture(e.pointerId); holding.add('scope'); } });
  sc.addEventListener('pointermove', (e) => {
    const y = e.clientY - sc.getBoundingClientRect().top;
    if (scopeDrag === 'gate') { const v = clamp(Math.round(toDb(y)), -80, -20); setKnob('gate', v).move(v); }
    else if (scopeDrag === 'comp') { const v = clamp(Math.round(toDb(y) * 2) / 2, -40, -6); P().comp.thr = v; sendComp.move(v); }
    else { scopeHover = nearLine(y); sc.style.cursor = scopeHover ? 'ns-resize' : 'default'; }
  });
  sc.addEventListener('pointerup', () => {
    if (scopeDrag === 'gate') setKnob('gate', P().knobs.gate).end(P().knobs.gate);
    if (scopeDrag === 'comp') sendComp.end(P().comp.thr);
    scopeDrag = null; holding.delete('scope');
  });
}

// ------------------------------------------------------------------ start
function connect() {
  fetch('/api/state').then((r) => { if (!r.ok) throw 0; return r.json(); }).then((st) => {
    S = st; online = true;
    if (S.ui && (S.ui.lang === 'zh' || S.ui.lang === 'en')) LANG = S.ui.lang;
    build();
    render();
    stats();
    autoPoll(true);
    const es = new EventSource('/events');
    es.addEventListener('meters', (e) => onMeters(JSON.parse(e.data)));
    es.addEventListener('devices', (e) => { D = JSON.parse(e.data); devicesUI(); });
    es.onopen = () => { online = true; };
    es.onerror = () => { online = false; };
    // leaving or reloading: close it first (Firefox otherwise logs the stream as cut off)
    addEventListener('beforeunload', () => es.close());
    setInterval(stats, 1000);
    requestAnimationFrame(tick);
  }).catch(() => {
    online = false;
    applyI18n();
    bannerUI();
    setTimeout(connect, 2000);
  });
}
connect();
