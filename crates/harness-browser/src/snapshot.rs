//! 页面元素收集与重定位脚本。
//!
//! ref → CSS 定位映射保存在 Rust 侧，
//! 注入脚本为一次性 IIFE，不向页面 `globalThis` 写入任何状态，防止不可信页面篡改映射。
use serde::Deserialize;

/// 收集可交互元素：返回 JSON 字符串（数组，元素含 ref/role/tag/name/value/locator）。
pub const COLLECT_JS: &str = r##"(() => {
  const selector = 'a[href],button,input,select,textarea,[role],[onclick],[contenteditable="true"],[contenteditable=""],[tabindex]';
  const out = [];
  let index = 0;
  for (const el of document.querySelectorAll(selector)) {
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden') continue;
    if (rect.width < 1 && rect.height < 1) continue;
    const tag = el.tagName.toLowerCase();
    const type = tag === 'input' ? (el.getAttribute('type') || 'text') : '';
    const role = el.getAttribute('role') || implicitRole(tag, type) || tag;
    const label = el.getAttribute('aria-label') || el.getAttribute('placeholder') || el.getAttribute('title') || '';
    const text = (el.innerText || '').trim().slice(0, 120);
    const name = (label || text || '').trim();
    const locator = buildLocator(el);
    if (!locator) continue;
    const isField = tag === 'input' || tag === 'textarea' || tag === 'select';
    out.push({
      ref: 'e' + (++index),
      role,
      tag,
      name,
      value: isField ? String(el.value ?? '') : undefined,
      locator
    });
  }
  function implicitRole(tag, type) {
    if (tag === 'a') return 'link';
    if (tag === 'button' || (tag === 'input' && ['submit', 'button', 'reset'].includes(type))) return 'button';
    if (tag === 'input' && type === 'checkbox') return 'checkbox';
    if (tag === 'input' && type === 'radio') return 'radio';
    if (tag === 'select') return 'combobox';
    if (tag === 'textarea' || tag === 'input') return 'textbox';
    if (/^h[1-6]$/.test(tag)) return 'heading';
    return '';
  }
  function buildLocator(el) {
    if (el.id) {
      const css = '#' + CSS.escape(el.id);
      if (unique(css, el)) return css;
    }
    const name = el.getAttribute('name');
    if (name) {
      const css = el.tagName.toLowerCase() + '[name="' + name.replace(/\\/g, '\\\\').replace(/"/g, '\\"') + '"]';
      if (unique(css, el)) return css;
    }
    const path = [];
    let node = el;
    while (node && node.nodeType === 1 && node !== document.documentElement) {
      let part = node.tagName.toLowerCase();
      const parent = node.parentElement;
      if (parent) {
        const same = Array.from(parent.children).filter((c) => c.tagName === node.tagName);
        if (same.length > 1) part += ':nth-of-type(' + (same.indexOf(node) + 1) + ')';
      }
      path.unshift(part);
      node = parent;
    }
    const css = path.length ? path.join('>') : 'body';
    return unique(css, el) ? css : null;
  }
  function unique(css, el) {
    const found = document.querySelectorAll(css);
    return found.length === 1 && found[0] === el;
  }
  return JSON.stringify(out);
})()"##;

/// 重定位函数：传入 CSS locator，滚动至元素可见并返回视口坐标 JSON 字符串。
pub const LOCATE_JS: &str = r##"((locator) => {
  const el = document.querySelector(locator);
  if (!el) return null;
  el.scrollIntoView({ block: 'center' });
  const rect = el.getBoundingClientRect();
  return JSON.stringify({ x: rect.x, y: rect.y, w: rect.width, h: rect.height, tag: el.tagName.toLowerCase() });
})"##;

/// 收集脚本的原始输出元素（locator 仅存在于 Rust 侧映射，不进入工具结果）。
#[derive(Debug, Clone, Deserialize)]
pub struct RawElement {
    #[serde(rename = "ref")]
    pub r#ref: String,
    pub role: String,
    pub tag: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: Option<String>,
    pub locator: String,
}

/// 重定位后的元素几何信息（视口坐标）。
#[derive(Debug, Clone, Deserialize)]
pub struct Located {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub tag: String,
}
