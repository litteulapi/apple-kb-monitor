// SPDX-License-Identifier: GPL-2.0-or-later
// Reader/editor for the daemon's config.toml. parse() reads TOML 1.0 (as the
// daemon's `toml` crate does, #12); set() changes only the value of one key
// and keeps every other line, comment and unknown key as it is.
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

// Section header "[name]" (spaces allowed inside the brackets), else null.
// Used by set() only.
function header(t) {
    const h = /^\[\s*([A-Za-z0-9_.-]+)\s*\]$/.exec(t);
    return h ? h[1] : null;
}

// ── Reader (#12) ──
// A TOML 1.0 reader, so that the module shows what the daemon reads: the
// daemon parses config.toml with the `toml` crate, and this page used to have
// its own line-by-line subset. The rules on tables (defined twice, extended
// by dotted keys, inline tables frozen...) follow Python's tomllib, the oracle
// of kcm/tests/toml_js_test.py. A document that is not valid TOML is read
// again statement by statement, like akm-core/src/config.rs does: one bad
// line costs one warning, never the whole file.

function SyntaxErr(pos, message) {
    this.name = "TomlSyntaxError";
    this.pos = pos;
    this.message = message;
}
SyntaxErr.prototype = Object.create(Error.prototype);

// Path flags, as tomllib's Flags: FROZEN (inline table or array, recursive)
// and EXPLICIT (declared by a [header], or opened by dotted keys).
const FROZEN = 0;
const EXPLICIT = 1;

function Flags() {
    this.root = Object.create(null);
    this.pending = [];
}
Flags.prototype.node = function (key, create) {
    let cont = this.root;
    let n = null;
    for (let i = 0; i < key.length; ++i) {
        n = cont[key[i]];
        if (n === undefined) {
            if (!create) return null;
            n = cont[key[i]] = { flags: [false, false], rec: [false, false], nested: Object.create(null) };
        }
        cont = n.nested;
    }
    return n;
};
Flags.prototype.set = function (key, flag, recursive) {
    (recursive ? this.node(key, true).rec : this.node(key, true).flags)[flag] = true;
};
Flags.prototype.unsetAll = function (key) {
    const parent = key.length > 1 ? this.node(key.slice(0, -1), false) : null;
    const cont = key.length > 1 ? (parent ? parent.nested : null) : this.root;
    if (cont) delete cont[key[key.length - 1]];
};
Flags.prototype.is = function (key, flag) {
    if (key.length === 0) return false;
    let cont = this.root;
    for (let i = 0; i < key.length - 1; ++i) {
        const n = cont[key[i]];
        if (n === undefined) return false;
        if (n.rec[flag]) return true;
        cont = n.nested;
    }
    const last = cont[key[key.length - 1]];
    return last !== undefined && (last.flags[flag] || last.rec[flag]);
};
Flags.prototype.finalize = function () {
    for (let i = 0; i < this.pending.length; ++i) this.set(this.pending[i], EXPLICIT, false);
    this.pending = [];
};

// Values: {t: "string"|"integer"|"float"|"boolean"|"datetime"|"array", v}
// or a table (a plain object without prototype, tagged by isTable()).
const TABLE = "\u0000table";
function newTable() {
    const t = Object.create(null);
    Object.defineProperty(t, TABLE, { value: true });
    return t;
}
function isTable(x) {
    return x !== null && typeof x === "object" && x[TABLE] === true;
}
function isList(x) {
    return Array.isArray(x);   // array of tables
}

// Get or create the table at `key` below `root`; the last item of an array
// of tables is entered unless `noLists`. Throws a string on a value.
function nest(root, key, noLists) {
    let cont = root;
    for (let i = 0; i < key.length; ++i) {
        let next = cont[key[i]];
        if (next === undefined) next = cont[key[i]] = newTable();
        if (!noLists && isList(next)) next = next[next.length - 1];
        if (!isTable(next)) throw "Cannot overwrite a value";
        cont = next;
    }
    return cont;
}

function Reader(src) {
    this.s = src;
    this.i = 0;
}
const R = Reader.prototype;
R.fail = function (message) { throw new SyntaxErr(this.i, message); };
R.peek = function (n) { return this.s.substr(this.i, n || 1); };
R.ws = function () {
    while (this.i < this.s.length && (this.s[this.i] === " " || this.s[this.i] === "\t")) this.i += 1;
};
// A comment, if any; control characters other than tab are refused.
R.comment = function () {
    if (this.s[this.i] !== "#") return;
    this.i += 1;
    while (this.i < this.s.length && this.s[this.i] !== "\n") {
        const c = this.s.charCodeAt(this.i);
        if (c === 0x7f || (c < 0x20 && c !== 0x09 && !(c === 0x0d && this.s[this.i + 1] === "\n")))
            this.fail("control character in a comment");
        this.i += 1;
    }
};
R.newline = function () {
    if (this.s[this.i] === "\n") { this.i += 1; return true; }
    if (this.s[this.i] === "\r" && this.s[this.i + 1] === "\n") { this.i += 2; return true; }
    return false;
};
// End of a statement: spaces, an optional comment, then a newline or the end.
R.eol = function () {
    this.ws();
    this.comment();
    if (this.i < this.s.length && !this.newline()) this.fail("expected the end of the line");
};
// Inside arrays: spaces, comments and newlines.
R.wsnl = function () {
    for (;;) {
        this.ws();
        this.comment();
        if (!this.newline()) return;
    }
};

R.keyPart = function () {
    const c = this.s[this.i];
    if (c === '"') return this.basic(false);
    if (c === "'") return this.literal(false);
    const m = /^[A-Za-z0-9_-]+/.exec(this.s.slice(this.i, this.i + 256));
    if (!m) this.fail("invalid key");
    this.i += m[0].length;
    return m[0];
};
R.key = function () {
    const parts = [this.keyPart()];
    for (;;) {
        this.ws();
        if (this.s[this.i] !== ".") return parts;
        this.i += 1;
        this.ws();
        parts.push(this.keyPart());
    }
};

function badChar(c, multi) {
    return c === 0x7f || (c < 0x20 && c !== 0x09 && !(multi && c === 0x0a));
}
R.escape = function () {
    const c = this.s[this.i];
    const simple = { b: "\b", t: "\t", n: "\n", f: "\f", r: "\r", '"': '"', "\\": "\\" };
    if (simple[c] !== undefined) { this.i += 1; return simple[c]; }
    if (c === "u" || c === "U") {
        const len = c === "u" ? 4 : 8;
        const hex = this.s.substr(this.i + 1, len);
        if (hex.length !== len || !/^[0-9A-Fa-f]+$/.test(hex)) this.fail("invalid unicode escape");
        const cp = parseInt(hex, 16);
        if (cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff)) this.fail("escape is not a unicode scalar value");
        this.i += 1 + len;
        return String.fromCodePoint(cp);
    }
    this.fail("invalid escape");
};
// "..." or, when multi, """...""".
R.basic = function (multi) {
    this.i += multi ? 3 : 1;
    if (multi) this.newline();
    let out = "";
    for (;;) {
        if (this.i >= this.s.length) this.fail("unterminated string");
        const ch = this.s[this.i];
        if (multi && this.s.substr(this.i, 3) === '"""') {
            // up to two quotes may stand just before the closing delimiter
            let n = 3;
            while (n < 5 && this.s[this.i + n] === '"') n += 1;
            out += '""'.slice(0, n - 3);
            this.i += n;
            return out;
        }
        if (!multi && ch === '"') { this.i += 1; return out; }
        if (ch === "\\") {
            this.i += 1;
            if (multi) {
                // line-ending backslash: drop the newline and the blanks after it
                const save = this.i;
                this.ws();
                if (this.newline()) {
                    for (;;) {
                        this.ws();
                        if (!this.newline()) break;
                    }
                    continue;
                }
                this.i = save;
            }
            out += this.escape();
            continue;
        }
        if (multi && ch === "\r" && this.s[this.i + 1] === "\n") { out += "\n"; this.i += 2; continue; }
        if (badChar(ch.charCodeAt(0), multi)) this.fail("control character in a string");
        out += ch;
        this.i += 1;
    }
};
// '...' or, when multi, '''...'''.
R.literal = function (multi) {
    this.i += multi ? 3 : 1;
    if (multi) this.newline();
    let out = "";
    for (;;) {
        if (this.i >= this.s.length) this.fail("unterminated string");
        const ch = this.s[this.i];
        if (multi && this.s.substr(this.i, 3) === "'''") {
            let n = 3;
            while (n < 5 && this.s[this.i + n] === "'") n += 1;
            out += "''".slice(0, n - 3);
            this.i += n;
            return out;
        }
        if (!multi && ch === "'") { this.i += 1; return out; }
        if (multi && ch === "\r" && this.s[this.i + 1] === "\n") { out += "\n"; this.i += 2; continue; }
        if (badChar(ch.charCodeAt(0), multi)) this.fail("control character in a string");
        out += ch;
        this.i += 1;
    }
};

function daysIn(y, m) {
    if (m === 2) return (y % 4 === 0 && y % 100 !== 0) || y % 400 === 0 ? 29 : 28;
    return [4, 6, 9, 11].indexOf(m) >= 0 ? 30 : 31;
}
const DATE = /^(\d{4})-(\d{2})-(\d{2})/;
const TIME = /^(\d{2}):(\d{2}):(\d{2})(\.\d+)?/;
R.datetime = function () {
    const rest = this.s.slice(this.i, this.i + 64);
    let text = "";
    const d = DATE.exec(rest);
    if (d) {
        const y = +d[1], mo = +d[2], da = +d[3];
        if (mo < 1 || mo > 12 || da < 1 || da > daysIn(y, mo)) this.fail("invalid date");
        text = d[0];
        const after = rest.slice(text.length);
        const t = /^[Tt ]/.test(after) ? TIME.exec(after.slice(1)) : null;
        if (t) text += after[0] + t[0];
        else if (/^[Tt]/.test(after)) this.fail("invalid date-time");
        if (t) {
            const off = /^([Zz]|[+-](\d{2}):(\d{2}))/.exec(rest.slice(text.length));
            if (off) {
                if (off[2] !== undefined && (+off[2] > 23 || +off[3] > 59)) this.fail("invalid offset");
                text += off[0];
            }
        }
    }
    const t = d ? TIME.exec(text.slice(11)) : TIME.exec(rest);
    if (!d && t) text = t[0];
    if (t && (+t[1] > 23 || +t[2] > 59 || +t[3] > 59)) this.fail("invalid time");
    if (text === "") return null;
    this.i += text.length;
    return { t: "datetime", v: text };
};
R.number = function () {
    const rest = this.s.slice(this.i, this.i + 512);
    const m = /^[0-9A-Za-z_.+-]+/.exec(rest);
    if (!m) this.fail("invalid value");
    const w = m[0];
    let r;
    if ((r = /^0x([0-9A-Fa-f](_?[0-9A-Fa-f])*)$/.exec(w))) return this.took(w, "integer", parseInt(r[1].replace(/_/g, ""), 16));
    if ((r = /^0o([0-7](_?[0-7])*)$/.exec(w))) return this.took(w, "integer", parseInt(r[1].replace(/_/g, ""), 8));
    if ((r = /^0b([01](_?[01])*)$/.exec(w))) return this.took(w, "integer", parseInt(r[1].replace(/_/g, ""), 2));
    if (/^[+-]?(0|[1-9](_?[0-9])*)$/.test(w)) return this.took(w, "integer", Number(w.replace(/_/g, "")));
    if (/^[+-]?(0|[1-9](_?[0-9])*)(\.[0-9](_?[0-9])*)?([eE][+-]?[0-9](_?[0-9])*)?$/.test(w) && /[.eE]/.test(w))
        return this.took(w, "float", Number(w.replace(/_/g, "")));
    if ((r = /^([+-]?)(inf|nan)$/.exec(w))) return this.took(w, "float", r[2] === "nan" ? NaN : (r[1] === "-" ? -Infinity : Infinity));
    this.fail("invalid value");
};
R.took = function (w, t, v) {
    this.i += w.length;
    return { t: t, v: v };
};
R.value = function () {
    const c = this.s[this.i];
    if (c === '"') return { t: "string", v: this.s.substr(this.i, 3) === '"""' ? this.basic(true) : this.basic(false) };
    if (c === "'") return { t: "string", v: this.s.substr(this.i, 3) === "'''" ? this.literal(true) : this.literal(false) };
    if (c === "[") return this.array();
    if (c === "{") return this.inlineTable();
    const word = /^[a-z]+/.exec(this.s.slice(this.i, this.i + 6));
    if (word && (word[0] === "true" || word[0] === "false")) {
        this.i += word[0].length;
        return { t: "boolean", v: word[0] === "true" };
    }
    if (/^\d{4}-\d{2}-\d{2}|^\d{2}:\d{2}/.test(this.s.slice(this.i, this.i + 10))) return this.datetime();
    return this.number();
};
R.array = function () {
    this.i += 1;
    const out = [];
    for (;;) {
        this.wsnl();
        if (this.s[this.i] === "]") { this.i += 1; return { t: "array", v: out }; }
        out.push(this.value());
        this.wsnl();
        const c = this.s[this.i];
        if (c === "]") { this.i += 1; return { t: "array", v: out }; }
        if (c !== ",") this.fail("expected , or ] in an array");
        this.i += 1;
    }
};
R.inlineTable = function () {
    this.i += 1;
    const table = newTable();
    const flags = new Flags();
    this.ws();
    if (this.s[this.i] === "}") { this.i += 1; return table; }
    for (;;) {
        const key = this.key();
        if (this.s[this.i] !== "=") this.fail("expected = after a key");
        this.i += 1;
        this.ws();
        const v = this.value();
        if (flags.is(key, FROZEN)) this.fail("Cannot mutate immutable namespace");
        let n;
        try { n = nest(table, key.slice(0, -1), true); } catch (e) { this.fail(e); }
        const stem = key[key.length - 1];
        if (stem in n) this.fail("Cannot overwrite a value");
        n[stem] = v;
        if (isTable(v) || v.t === "array") flags.set(key, FROZEN, true);
        this.ws();
        const c = this.s[this.i];
        if (c === "}") { this.i += 1; return table; }
        if (c !== ",") this.fail("expected , or } in an inline table");
        this.i += 1;
        this.ws();
    }
};

function lineAt(src, pos) {
    let n = 1;
    for (let i = 0; i < pos && i < src.length; ++i) if (src[i] === "\n") n += 1;
    return n;
}

// Strict reading of the whole document: {root, lines} or a SyntaxErr thrown.
// `lines` maps "a.b.key" to the line of the key.
function document(src) {
    const rd = new Reader(src);
    const root = newTable();
    const flags = new Flags();
    const lines = {};
    let headerKey = [];
    for (;;) {
        rd.wsnl();
        if (rd.i >= src.length) break;
        const at = rd.i;
        if (src[rd.i] === "[") {
            const list = src[rd.i + 1] === "[";
            rd.i += list ? 2 : 1;
            rd.ws();
            const key = rd.key();
            if (list ? src.substr(rd.i, 2) !== "]]" : src[rd.i] !== "]") rd.fail("unterminated table header");
            rd.i += list ? 2 : 1;
            flags.finalize();
            if (list) {
                if (flags.is(key, FROZEN)) rd.fail("Cannot mutate immutable namespace");
                flags.unsetAll(key);
                flags.set(key, EXPLICIT, false);
                try {
                    const parent = nest(root, key.slice(0, -1), false);
                    const last = key[key.length - 1];
                    if (last in parent) {
                        if (!isList(parent[last])) throw "Cannot overwrite a value";
                        parent[last].push(newTable());
                    } else {
                        parent[last] = [newTable()];
                    }
                } catch (e) {
                    rd.i = at;
                    rd.fail(e);
                }
            } else {
                if (flags.is(key, EXPLICIT) || flags.is(key, FROZEN)) { rd.i = at; rd.fail("Cannot declare a table twice"); }
                flags.set(key, EXPLICIT, false);
                try { nest(root, key, false); } catch (e) { rd.i = at; rd.fail(e); }
            }
            headerKey = key;
            rd.eol();
            continue;
        }
        const key = rd.key();
        if (src[rd.i] !== "=") rd.fail("expected = after a key");
        rd.i += 1;
        rd.ws();
        const v = rd.value();
        const abs = headerKey.concat(key);
        for (let k = 1; k < key.length; ++k) {
            const cont = headerKey.concat(key.slice(0, k));
            if (flags.is(cont, EXPLICIT)) { rd.i = at; rd.fail("Cannot redefine a namespace"); }
            flags.pending.push(cont);
        }
        const parentKey = abs.slice(0, -1);
        if (flags.is(parentKey, FROZEN)) { rd.i = at; rd.fail("Cannot mutate immutable namespace"); }
        let n;
        try { n = nest(root, parentKey, false); } catch (e) { rd.i = at; rd.fail(e); }
        const stem = key[key.length - 1];
        if (stem in n) { rd.i = at; rd.fail("Cannot overwrite a value"); }
        if (isTable(v) || v.t === "array") flags.set(abs, FROZEN, true);
        n[stem] = v;
        lines[abs.join(".")] = lineAt(src, at);
        rd.eol();
    }
    return { root: root, lines: lines };
}

// The JavaScript value of a parsed value: tables as objects, arrays as
// arrays, datetimes as their text.
function plain(x) {
    if (isList(x)) return x.map(plain);
    if (isTable(x)) {
        const o = {};
        for (const k in x) o[k] = plain(x[k]);
        return o;
    }
    if (x.t === "array") return x.v.map(plain);
    return x.v;
}

// Every value that is not a table, by "section.key" (".key" outside any
// section, "a.b.key" in [a.b]); arrays of tables are left out.
function flatten(table, prefix, values, kinds) {
    for (const k in table) {
        const x = table[k];
        if (isList(x)) continue;
        if (isTable(x)) flatten(x, prefix === "" ? k : prefix + "." + k, values, kinds);
        else {
            const name = prefix + "." + k;
            values[name] = plain(x);
            kinds[name] = x.t === "array" && x.v.every(function (e) { return e.t === "integer"; }) ? "integers" : x.t;
        }
    }
}

// text -> { values: {"section.key": value}, kinds: {"section.key": type},
//           lines: {"section.key": line}, warnings: ["line N"], strict: bool }
// `kinds` gives the TOML type of each value ("integer", "float", "string",
// "boolean", "datetime", "array", or "integers" for an array of integers),
// which JavaScript numbers lose.
function parse(text) {
    const src = String(text).replace(/^﻿/, "");
    const values = {};
    const kinds = {};
    const lines = {};
    const warnings = [];
    try {
        const doc = document(src);
        flatten(doc.root, "", values, kinds);
        for (const k in values) lines[k] = doc.lines[k.charAt(0) === "." ? k.slice(1) : k];
        return { values: values, kinds: kinds, lines: lines, warnings: warnings, strict: true };
    } catch (e) {
        if (!(e instanceof SyntaxErr)) throw e;
    }
    // Not valid TOML: statement by statement (akm-core/src/config.rs,
    // Reader::statements). Each `key = value` is read by the strict reader;
    // a header only changes the current section.
    const all = src.split("\n");
    let section = "";
    for (let n = 0; n < all.length; ++n) {
        const first = n;
        let line = stripComment(all[n]).trim();
        if (line === "") continue;
        const eq = line.indexOf("=");
        if (eq > 0) {
            const v = line.slice(eq + 1).trim();
            if (v[0] === "[" && v[v.length - 1] !== "]") {
                let closed = false;
                while (n + 1 < all.length) {
                    n += 1;
                    line += " " + stripComment(all[n]).trim();
                    if (line[line.length - 1] === "]") { closed = true; break; }
                }
                if (!closed) { warnings.push("line " + (first + 1)); continue; }
            }
        }
        if (line[0] === "[" && line[line.length - 1] === "]") {
            section = line.slice(1, -1).trim();
            continue;
        }
        if (eq <= 0) { warnings.push("line " + (first + 1)); continue; }
        let doc;
        try {
            doc = document(line);
        } catch (e) {
            if (!(e instanceof SyntaxErr)) throw e;
            warnings.push("line " + (first + 1));
            continue;
        }
        for (const k in doc.root) {
            const x = doc.root[k];
            if (isTable(x) || isList(x)) continue;
            const name = section + "." + k;
            values[name] = plain(x);
            kinds[name] = x.t === "array" && x.v.every(function (e) { return e.t === "integer"; }) ? "integers" : x.t;
            lines[name] = first + 1;
        }
    }
    return { values: values, kinds: kinds, lines: lines, warnings: warnings, strict: false };
}

// The value of `key` if the daemon would take it as `want` ("boolean",
// "integer", "float" -- an integer is accepted --, "string", "integers"),
// else undefined. Strict like serde in akm-core/src/config.rs: an integer is
// not a boolean, a string is not a number.
function typed(p, key, want) {
    const k = p.kinds[key];
    if (k === undefined) return undefined;
    const ok = k === want || (want === "float" && k === "integer") || (want === "integers" && k === "array" && p.values[key].length === 0);
    if (!ok) return undefined;
    if (want === "float" && !isFinite(p.values[key])) return undefined;
    return p.values[key];
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

// The lines of `src`, each with its own terminator ("\n", "\r\n", or ""
// for a last line without one), so that every line not edited is given back
// byte for byte (#284).
function splitLines(src) {
    const out = [];
    let i = 0;
    while (i < src.length) {
        const n = src.indexOf("\n", i);
        if (n < 0) {
            out.push({ body: src.slice(i), eol: "" });
            break;
        }
        let body = src.slice(i, n);
        let eol = "\n";
        if (body.length > 0 && body[body.length - 1] === "\r") {
            body = body.slice(0, -1);
            eol = "\r\n";
        }
        out.push({ body: body, eol: eol });
        i = n + 1;
    }
    return out;
}

// Last line of the `key = value` statement that starts at line i: the
// shortest run of lines the strict reader takes as one statement (a value
// may span lines: array, multi-line string). i when none does (a bad line).
function statementEnd(lines, i) {
    let text = "";
    for (let end = i; end < lines.length && end < i + 400; ++end) {
        text += lines[end].body + "\n";
        try {
            document(text);
            return end;
        } catch (e) {
            // not complete yet
        }
    }
    return i;
}

function parsesAlone(body) {
    try {
        document(body + "\n");
        return true;
    } catch (e) {
        return false;
    }
}

function sameValues(a, b) {
    return JSON.stringify(a) === JSON.stringify(b);
}

// Set section.key = value in text; returns the new text. Only the lines of
// that one statement change (or one line is added); every other byte is
// kept: line endings line by line (LF, CRLF, none at the end), comments,
// other sections. The result is read back: if any other value changed, or
// the value is not the one asked, nothing is returned (UnsupportedForm).
// Throws UnsupportedForm when `section` is an array of tables
// ([[section]]), an inline table or dotted keys (`section = {...}`,
// `section.key = ...`), a quoted header, when `key` is quoted in it, or
// when its value is a string written on several lines (#287): the file must
// then be edited by hand (#258).
function set(text, section, key, value) {
    const src = String(text);
    const eol = src.indexOf("\r\n") >= 0 ? "\r\n" : "\n";
    const lines = splitLines(src);
    const refuse = function (i, what) {
        throw new UnsupportedForm("line " + (i + 1) + ": " + what + " — edit the file by hand");
    };
    let cur = "";
    let sectionStart = -1;
    let lastInSection = -1;
    let found = -1;
    let foundEnd = -1;
    for (let i = 0; i < lines.length; ++i) {
        const t = stripComment(lines[i].body).trim();
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
        // the lines of a value written on several lines are not statements
        const end = eq > 0 ? statementEnd(lines, i) : i;
        if (cur === "" && parts[0] === section && rawKey !== "") {
            refuse(i, "inline table or dotted keys for [" + section + "]");
        }
        if (cur === section) {
            lastInSection = end;
            if (found < 0) {
                if (parts.length === 1 && parts[0] === key && rawKey !== key) refuse(i, "quoted key " + rawKey);
                if (parts.length > 1 && parts[0] === key) refuse(i, "dotted key " + rawKey);
                if (rawKey === key) { found = i; foundEnd = end; }
            }
        }
        i = end;
    }
    const lineOf = function (body, ending) { return { body: body, eol: ending }; };
    if (found >= 0) {
        const i = found;
        const m = /^(\s*)([A-Za-z0-9_-]+)(\s*=\s*)(.*)$/.exec(lines[i].body);
        const rest = lines[i].body.slice(m[1].length + m[2].length + m[3].length);
        // keep the comment after the value (on the last line of a value
        // written on several lines; the comments inside it are not kept)
        const last = foundEnd === i ? rest : lines[foundEnd].body;
        const comment = last.slice(stripComment(last).length).trim();
        lines.splice(i, foundEnd - i + 1,
                     lineOf(m[1] + m[2] + m[3] + literal(value) + (comment !== "" ? "  " + comment : ""), lines[foundEnd].eol));
    } else if (sectionStart >= 0) {
        const after = lines[lastInSection];
        const ending = after.eol;
        if (after.eol === "") after.eol = eol;
        lines.splice(lastInSection + 1, 0, lineOf(key + " = " + literal(value), ending));
    } else {
        const ending = lines.length > 0 ? lines[lines.length - 1].eol : eol;
        if (lines.length > 0 && lines[lines.length - 1].eol === "") lines[lines.length - 1].eol = eol;
        if (lines.length > 0 && lines[lines.length - 1].body.trim() !== "") lines.push(lineOf("", eol));
        lines.push(lineOf("[" + section + "]", eol));
        lines.push(lineOf(key + " = " + literal(value), ending));
    }
    const out = lines.map(function (l) { return l.body + l.eol; }).join("");
    // Read back (#284): every other value as before, this one as asked.
    const before = parse(src);
    const after = parse(out);
    const name = section + "." + key;
    for (const k in before.values) {
        if (k !== name && !sameValues(before.values[k], after.values[k])) {
            throw new UnsupportedForm(k + " would change too — edit the file by hand");
        }
    }
    for (const k in after.values) {
        if (k !== name && !(k in before.values)) {
            throw new UnsupportedForm(k + " would appear — edit the file by hand");
        }
    }
    if (!sameValues(after.values[name], value) || (before.strict && !after.strict)) {
        throw new UnsupportedForm(name + " could not be written safely — edit the file by hand");
    }
    return out;
}
