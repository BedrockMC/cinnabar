package control

import "regexp"

// maxLoggedError bounds a logged service error.
const maxLoggedError = 400

var secretPatterns = []*regexp.Regexp{
	// JWTs (MCToken, PlayFab entity and XSTS tokens) and XBL3.0 header values.
	regexp.MustCompile(`eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+(\.[A-Za-z0-9_-]+)?`),
	regexp.MustCompile(`XBL3\.0 x=[^\s;"]+;[^\s"]+`),
	regexp.MustCompile(`(?i)(authorization|token|ticket|session[-_]?id)(["':= ]+)[^\s",}]+`),
}

// RedactError is err's message with tokens replaced and its length bounded,
// safe to write to the core log.
func RedactError(err error) string {
	if err == nil {
		return ""
	}
	message := err.Error()
	for _, pattern := range secretPatterns[:2] {
		message = pattern.ReplaceAllString(message, "<redacted>")
	}
	message = secretPatterns[2].ReplaceAllString(message, "$1$2<redacted>")
	if len(message) > maxLoggedError {
		message = message[:maxLoggedError] + "…"
	}
	return message
}
