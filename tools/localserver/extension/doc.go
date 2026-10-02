// Package extension is the server half of the Cinnabar extension wire (PR #34): Go mirrors of
// the Rust offer, marker, handshake, manifest and envelope types, their canonical bytes, Ed25519
// signing and the client's envelope validation.
//
// Rust is the source of truth. Canonical JSON is the exact compact serde_json encoding of the
// Rust structs, which encoding/json cannot produce (it escapes U+2028 and U+2029 and replaces
// invalid UTF-8), so Encode writes it directly and Decode reads it as strictly as serde_json does.
// The tests decode, re-encode and re-sign every golden that `cinnabar-cxb write-fixtures
// tools/localserver/extension/testdata` writes and require identical bytes.
package extension
