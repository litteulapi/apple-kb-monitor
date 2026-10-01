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

// text -> { values: {"section.key": value}, warnings: [string] }
function parse(text) {
    const values = {};
    const warnings = [];
    const lines = String(text).replace(/^\uFEFF/, "").split("\n");
    let section = "";
    for (let n = 0; n < lines.length; ++n) {
        let line = stripComment(lines[n]).trim();
        if (line === "") continue;
        const h = /^\[([A-Za-z0-9_.-]+)\]$/.exec(line);
        if (h) { section = h[1]; continue; }
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

// Set section.key = value in text; returns the new text.
function set(text, section, key, value) {
    const lines = String(text).split("\n");
    if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
    let cur = "";
    let sectionStart = -1;
    let lastInSection = -1;
    for (let i = 0; i < lines.length; ++i) {
        const t = stripComment(lines[i]).trim();
        const h = /^\[([A-Za-z0-9_.-]+)\]$/.exec(t);
        if (h) {
            cur = h[1];
            if (cur === section) { sectionStart = i; lastInSection = i; }
            continue;
        }
        if (cur !== section || t === "") continue;
        lastInSection = i;
        const m = /^(\s*)([A-Za-z0-9_-]+)(\s*=\s*)(.*)$/.exec(lines[i]);
        if (m && m[2] === key) {
            // keep the comment after the value
            const rest = lines[i].slice(m[1].length + m[2].length + m[3].length);
            const bare = stripComment(rest);
            let end = i;
            // a multi-line array value ends at the line holding "]"
            if (bare.trim()[0] === "[" && bare.indexOf("]") < 0) {
                while (end + 1 < lines.length && stripComment(lines[end]).indexOf("]") < 0) end += 1;
            }
            const comment = end === i ? rest.slice(bare.length).trim() : "";
            lines.splice(i, end - i + 1, m[1] + m[2] + m[3] + literal(value) + (comment !== "" ? "  " + comment : ""));
            return lines.join("\n") + "\n";
        }
    }
    if (sectionStart >= 0) {
        lines.splice(lastInSection + 1, 0, key + " = " + literal(value));
    } else {
        if (lines.length > 0 && lines[lines.length - 1].trim() !== "") lines.push("");
        lines.push("[" + section + "]");
        lines.push(key + " = " + literal(value));
    }
    return lines.join("\n") + "\n";
}
