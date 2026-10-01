// Package targetpin reads carrier identities from the repository target manifest.
package targetpin

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

// LightHash finds the target manifest above the working directory and reads its light pin.
func LightHash() (string, error) {
	dir, err := os.Getwd()
	if err != nil {
		return "", err
	}
	for {
		data, err := os.ReadFile(filepath.Join(dir, "assets", "bedrock-target.json"))
		if err == nil {
			var target struct {
				Hashes map[string]string `json:"hashes"`
			}
			if err := json.Unmarshal(data, &target); err != nil {
				return "", err
			}
			hash := target.Hashes["light_registry"]
			if len(hash) != 64 {
				return "", fmt.Errorf("invalid light registry pin")
			}
			return hash, nil
		}
		if !os.IsNotExist(err) {
			return "", err
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "", fmt.Errorf("bedrock target manifest not found")
		}
		dir = parent
	}
}
