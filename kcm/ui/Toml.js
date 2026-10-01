// SPDX-License-Identifier: GPL-2.0-or-later
// Minimal reader/editor for the daemon's config.toml (the subset documented
// in docs/CONFIGURATION.md: tables, ints, floats, booleans, strings, arrays of
// ints, comments). Edits change only the value of one key and keep every
// other line, comment and unknown key as it is.
.pragma library

function stripComment(line) {
    let inStr = false;
    for (let i = 0; i < line.length; ++i) {
        const c = line[i];
        if (c === '"' && line[i - 1] !== "\\") inStr = !inStr;
        else if (c === "#" && !inStr) return line.slice(0, i);
    }
    return line;
}

function parseValue(v) {
    v = v.trim();
    if (v === "true") return true;
    if (v === "false") return false;
    if (v.length >= 2 && v[0] === '"' && v[v.length - 1] === '"') return v.slice(1, -1);
    if (v[0] === "[" && v[v.length - 1] === "]") {
        const inner = v.slice(1, -1).trim();
        if (inner === "") return [];
        const out = [];
        const parts = inner.split(",");
        for (let i = 0; i < parts.length; ++i) {
            const p = parts[i].trim();
            if (p === "") continue;
            const n = Number(p);
            if (!isFinite(n)) return undefined;
            out.push(n);
        }
        return out;
    }
    if (/^[+-]?\d+(\.\d+)?$/.test(v)) return Number(v);
    return undefined;
}

// Section header "[name]" (spaces allowed inside the brackets), else null.
function header(t) {
    const h = /^\[\s*([A-Za-z0-9_.-]+)\s*\]$/.exec(t);
    return h ? h[1] : null;
}

// text -> { values: {"section.key": value}, warnings: [string] }
function parse(text) {
    const values = {};
    const warnings = [];
    const lines = String(text).replace(/^\uFEFF/, "").replace(/\r\n/g, "\n").split("\n");
    let section = "";
    for (let n = 0; n < lines.length; ++n) {
        let line = stripComment(lines[n]).trim();
        if (line === "") continue;
        const h = header(line);
        if (h !== null) { section = h; continue; }
        const eq = line.indexOf("=");
        if (eq <= 0) { warnings.push("line " + (n + 1)); continue; }
        const key = line.slice(0, eq).trim();
        let raw = line.slice(eq + 1).trim();
        // multi-line array
        while (raw[0] === "[" && raw.indexOf("]") < 0 && n + 1 < lines.length) {
            n += 1;
            raw += " " + stripComment(lines[n]).trim();
        }
        const v = parseValue(raw);
        if (v === undefined) warnings.push("line " + (n + 1));
        else values[section + "." + key] = v;
    }
    return { values: values, warnings: warnings };
}

function literal(v) {
    if (typeof v === "boolean") return v ? "true" : "false";
    if (typeof v === "number") return String(v);
    if (Array.isArray(v)) return "[" + v.join(", ") + "]";
    return '"' + String(v).replace(/["\\]/g, "") + '"';
}

// Thrown by set() when the file writes `section` or `key` in a TOML form this
// editor does not rewrite (#258): editing it line by line would declare the
// key or the table a second time, which no TOML reader accepts.
function UnsupportedForm(message) {
    this.name = "UnsupportedForm";
    this.message = message;
}
UnsupportedForm.prototype = Object.create(Error.prototype);

function unquote(k) {
    k = k.trim();
    if (k.length >= 2 && (k[0] === '"' || k[0] === "'") && k[k.length - 1] === k[0]) return k.slice(1, -1);
    return k;
}

// Set section.key = value in text; returns the new text. Line endings (LF or
// CRLF) are kept. Throws UnsupportedForm when `section` is an array of tables
// ([[section]]), an inline table or dotted keys (`section = {...}`,
// `section.key = ...`), a quoted header, or when `key` is quoted in it: the
// file must then be edited by hand (#258).
function set(text, section, key, value) {
    const src = String(text);
    const eol = src.indexOf("\r\n") >= 0 ? "\r\n" : "\n";
    const lines = src.replace(/\r\n/g, "\n").split("\n");
    if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
    const refuse = function (i, what) {
        throw new UnsupportedForm("line " + (i + 1) + ": " + what + " — edit the file by hand");
    };
    let cur = "";
    let sectionStart = -1;
    let lastInSection = -1;
    let found = -1;
    for (let i = 0; i < lines.length; ++i) {
        const t = stripComment(lines[i]).trim();
        if (t === "") continue;
        if (t[0] === "[" && t[1] === "[") {
            const inner = t.replace(/^\[\[|\]\]$/g, "").split(".").map(unquote).join(".");
            if (inner === section) refuse(i, "[" + "[" + section + "]] (array of tables)");
            cur = "\u0000array";
            continue;
        }
        const h = header(t);
        if (h !== null) {
            cur = h;
            if (cur === section) { sectionStart = i; lastInSection = i; }
            continue;
        }
        if (t[0] === "[" && /\]$/.test(t) && t.indexOf("=") < 0) {
            // quoted header: ["alerts"], [ 'alerts' ]
            const inner = t.slice(1, -1).split(".").map(unquote).join(".");
            if (inner === section) refuse(i, "quoted table header " + t);
            cur = "\u0000quoted";
            continue;
        }
        const eq = t.indexOf("=");
        const rawKey = eq > 0 ? t.slice(0, eq).trim() : "";
        const parts = rawKey.split(".").map(unquote);
        if (cur === "" && parts[0] === section && rawKey !== "") {
            refuse(i, "inline table or dotted keys for [" + section + "]");
        }
        if (cur !== section) continue;
        lastInSection = i;
        if (found >= 0) continue;
        if (parts.length === 1 && parts[0] === key && rawKey !== key) refuse(i, "quoted key " + rawKey);
        if (parts.length > 1 && parts[0] === key) refuse(i, "dotted key " + rawKey);
        if (rawKey === key) found = i;
    }
    if (found >= 0) {
        const i = found;
        const m = /^(\s*)([A-Za-z0-9_-]+)(\s*=\s*)(.*)$/.exec(lines[i]);
        const rest = lines[i].slice(m[1].length + m[2].length + m[3].length);
        const bare = stripComment(rest);
        let end = i;
        // a multi-line array value ends at the line holding "]"
        if (bare.trim()[0] === "[" && bare.indexOf("]") < 0) {
            while (end + 1 < lines.length && stripComment(lines[end]).indexOf("]") < 0) end += 1;
        }
        // keep the comment after the value (on the line of "]" for a multi-line array)
        const last = end === i ? rest : lines[end];
        const comment = last.slice(stripComment(last).length).trim();
        lines.splice(i, end - i + 1, m[1] + m[2] + m[3] + literal(value) + (comment !== "" ? "  " + comment : ""));
    } else if (sectionStart >= 0) {
        lines.splice(lastInSection + 1, 0, key + " = " + literal(value));
    } else {
        if (lines.length > 0 && lines[lines.length - 1].trim() !== "") lines.push("");
        lines.push("[" + section + "]");
        lines.push(key + " = " + literal(value));
    }
    return lines.join(eol) + eol;
}
