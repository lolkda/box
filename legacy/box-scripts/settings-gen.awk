# Parse a flat settings.json object and emit shell assignments or a normalized JSON object.
# Supported values: strings and arrays of strings. This intentionally rejects nested objects.
BEGIN {
  mode = (mode == "" ? "shell" : mode)
  default_file = ARGV[1]
  user_file = ARGV[2]

  if (default_file == "") {
    fail("missing default settings file")
  }
  load_json(default_file)
  if (user_file != "") load_json(user_file)

  if (mode == "json") emit_json()
  else emit_shell()
  exit 0
}

function fail(message) {
  printf "settings-gen: %s\n", message > "/dev/stderr"
  exit 1
}

function peek() {
  return substr(json, pos, 1)
}

function skip_ws(   c) {
  while (pos <= length(json)) {
    c = peek()
    if (c != " " && c != "\t" && c != "\r" && c != "\n") break
    pos++
  }
}

function expect(ch) {
  skip_ws()
  if (peek() != ch) fail("expected " ch " at byte " pos)
  pos++
}

function parse_string(   c, e, out) {
  skip_ws()
  if (peek() != "\"") fail("expected string at byte " pos)
  pos++
  out = ""
  while (pos <= length(json)) {
    c = substr(json, pos, 1)
    pos++
    if (c == "\"") return out
    if (c != "\\") {
      out = out c
      continue
    }
    e = substr(json, pos, 1)
    pos++
    if (e == "n") out = out "\n"
    else if (e == "r") out = out "\r"
    else if (e == "t") out = out "\t"
    else if (e == "b") out = out "\b"
    else if (e == "f") out = out "\f"
    else if (e == "u") {
      # The settings schema only requires ordinary text. Preserve ASCII escapes
      # and use a replacement for non-ASCII code points that busybox awk cannot
      # portably encode.
      hex = substr(json, pos, 4)
      pos += 4
      if (hex !~ /^[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]$/) {
        fail("invalid unicode escape")
      }
      code = hex_to_dec(hex)
      if (code >= 32 && code < 127) out = out sprintf("%c", code)
      else out = out "?"
    } else out = out e
  }
  fail("unterminated string")
}

function hex_to_dec(hex,   i, c, n) {
  n = 0
  for (i = 1; i <= 4; i++) {
    c = tolower(substr(hex, i, 1))
    n = n * 16 + index("0123456789abcdef", c) - 1
  }
  return n
}

function parse_array(   c) {
  expect("[")
  val_count = 0
  skip_ws()
  if (peek() == "]") {
    pos++
    is_array = 1
    return
  }
  while (1) {
    skip_ws()
    if (peek() != "\"") fail("array items must be strings at byte " pos)
    val[++val_count] = parse_string()
    skip_ws()
    c = peek()
    if (c == ",") { pos++; continue }
    if (c == "]") { pos++; break }
    fail("expected , or ] at byte " pos)
  }
  is_array = 1
}

function parse_value() {
  skip_ws()
  if (peek() == "\"") {
    value = parse_string()
    is_array = 0
    return
  }
  if (peek() == "[") {
    parse_array()
    return
  }
  fail("expected string or array at byte " pos)
}

function parse_object(   c, key, i) {
  expect("{")
  skip_ws()
  if (peek() == "}") { pos++; return }
  while (1) {
    key = parse_string()
    if (allowed != "" && index(allowed, " " key " ") == 0) {
      fail("unknown settings key: " key)
    }
    expect(":")
    parse_value()
    if (key == "githubtoken") {
      if (is_array) fail("githubtoken must be a string")
      token_present = 1
      token_value = value
    } else if (is_array) {
      arr_len[key] = val_count
      for (i = 1; i <= val_count; i++) arr[key, i] = val[i]
    } else {
      scalar[key] = value
      if (key in arr_len) delete arr_len[key]
    }
    if (key != "githubtoken" && !(key in seen)) {
      seen[key] = 1
      order[++order_count] = key
    }
    skip_ws()
    c = peek()
    if (c == ",") { pos++; continue }
    if (c == "}") { pos++; break }
    fail("expected , or } at byte " pos)
  }
}

function load_json(path,   line) {
  json = ""
  while ((getline line < path) > 0) json = json line "\n"
  close(path)
  if (json == "") fail("empty settings file: " path)
  pos = 1
  parse_object()
  skip_ws()
  if (pos <= length(json)) fail("trailing data in " path)
}

function shq(s,   out, i, c) {
  out = ""
  for (i = 1; i <= length(s); i++) {
    c = substr(s, i, 1)
    if (c == "'") out = out "'\\''"
    else out = out c
  }
  return "'" out "'"
}

function json_quote(s,   out, i, c) {
  out = ""
  for (i = 1; i <= length(s); i++) {
    c = substr(s, i, 1)
    if (c == "\\") out = out "\\\\"
    else if (c == "\"") out = out "\\\""
    else if (c == "\n") out = out "\\n"
    else if (c == "\r") out = out "\\r"
    else if (c == "\t") out = out "\\t"
    else out = out c
  }
  return "\"" out "\""
}

function emit_shell(   i, j, key) {
  if (token_present) {
    print "settings_token_present=1"
    printf "settings_input_token=%s\n", shq(token_value)
  }
  for (i = 1; i <= order_count; i++) {
    key = order[i]
    if (key in arr_len) {
      printf "%s=(", key
      for (j = 1; j <= arr_len[key]; j++) {
        if (j > 1) printf " "
        printf "%s", shq(arr[key, j])
      }
      print ")"
    } else {
      printf "%s=%s\n", key, shq(scalar[key])
    }
  }
}

function emit_json(   i, j, key) {
  printf "{"
  for (i = 1; i <= order_count; i++) {
    key = order[i]
    if (i > 1) printf ","
    printf "%s:", json_quote(key)
    if (key in arr_len) {
      printf "["
      for (j = 1; j <= arr_len[key]; j++) {
        if (j > 1) printf ","
        printf "%s", json_quote(arr[key, j])
      }
      printf "]"
    } else {
      printf "%s", json_quote(scalar[key])
    }
  }
  print "}"
}
