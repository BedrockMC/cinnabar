package main

import (
	"bytes"
	"crypto/sha256"
	"errors"
	"fmt"
	"os"
)

// Dragonfly reports (emission 0, filter 15) for a state it has no block implementation
// for, so unimplemented translucent plants (glow lichen, cave vines, hanging roots, ...)
// inherited a full light filter. The pinned retail block table carries their real values.
const (
	unknownBlockEmission = 0
	unknownBlockFilter   = 15
)

// applyRetailLightCorrections replaces every unimplemented-block default in properties
// (emission | filter<<4, parallel to records) with the retail table's values and returns
// how many states changed. Names absent from the table keep their value.
func applyRetailLightCorrections(records []Record, properties []byte, retail map[string]PMMPLightProperties) (int, error) {
	if len(records) != len(properties) {
		return 0, errors.New("light property count does not match records")
	}
	changed := 0
	for index, record := range records {
		if record.Name == retailReservedName {
			continue
		}
		current := properties[index]
		if current&0x0f != unknownBlockEmission || current>>4 != unknownBlockFilter {
			continue
		}
		exact, ok := retail[record.Name]
		if !ok {
			continue
		}
		emission, filter, err := checkedPMMPLight(record.Name, exact)
		if err != nil {
			return 0, err
		}
		next := emission | filter<<4
		if next != current {
			properties[index] = next
			changed++
		}
	}
	return changed, nil
}

// relightV2168 rewrites an existing v2168 LREG with the retail corrections applied,
// bound to the same BREG; it returns the new LREG bytes and the changed-state count.
func relightV2168(bregPath, lregPath, retailPath string) ([]byte, int, error) {
	breg, err := os.ReadFile(bregPath)
	if err != nil {
		return nil, 0, fmt.Errorf("read BREG: %w", err)
	}
	_, records, err := decodeBREGRecords(breg, v2168BlockProtocol)
	if err != nil {
		return nil, 0, err
	}
	lreg, err := os.ReadFile(lregPath)
	if err != nil {
		return nil, 0, fmt.Errorf("read LREG: %w", err)
	}
	properties, err := decodeLREGProperties(lreg, breg, v2168BlockProtocol, len(records))
	if err != nil {
		return nil, 0, err
	}
	retail, err := readPMMPLightProperties(retailPath)
	if err != nil {
		return nil, 0, err
	}
	changed, err := applyRetailLightCorrections(records, properties, retail)
	if err != nil {
		return nil, 0, err
	}
	encoded, err := encodeResolvedLightRegistryForProtocol(v2168BlockProtocol, breg, records, properties)
	if err != nil {
		return nil, 0, err
	}
	if bytes.Equal(encoded, lreg) && changed != 0 {
		return nil, 0, errors.New("relight produced no byte change")
	}
	_ = sha256.Size
	return encoded, changed, nil
}
