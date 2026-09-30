package main

import (
	"bytes"
	"crypto/sha256"
	"debug/pe"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"unicode/utf8"

	"github.com/df-mc/dragonfly/server/world"
)

const (
	v2193BiomeRecordSize       = 24
	v2193BiomeSourceCount      = 89
	v2193RetailBiomeCount      = 89
	v2193PMMPBiomeCount        = 88
	v2193BiomeTableOffset      = 0x0a80_d518
	v2193BiomeRetailTableBytes = v2193RetailBiomeCount * v2193BiomeRecordSize
	v2193BiomeTableBytes       = v2193BiomeSourceCount * v2193BiomeRecordSize
	v2193ImageBase             = 0x1_4000_0000

	v2193BDSExecutableSHA256 = "19c88569af2e4b7d984e999055a31cbcb0799dacf8bbbf7371eda42f5772a443"
	v2193BiomeTableSHA256    = "39812048dfedc2b5dd0c22043297de7869f4352fed7550b377b69e06854fc5b6"
	v2193RetailTableSHA256   = v2193BiomeTableSHA256
	v2193PMMPBiomeMapSHA256  = "4f27df3f1e58476fc65e337f7cf3e275f65a98b6c40ea46c31b24016b85e0052"
	v2193BiomeAllowSHA256    = "6127c74c17455273bb5226f1e05e98709bc247c05a0137a8827cb97756c3b198"

	// The BiomeDefinitionList captured from the Linux build of the same
	// release names exactly the retail allowlist.
	v2193CaptureArchiveSHA256 = "f6348d84fa714d04ca194f207e89453ca6bba0a1359396475271a52a150471c6"
	v2193CaptureBinarySHA256  = "0a490c711d4a2ce075debcd979e8862b7eaba008373f9970f0edbb67f98f9207"

	v2193BiomeOutputPath     = "crates/assets/data/biome-registry-v2193.bin"
	v2193BiomeAllowlistPath  = "crates/protocol/data/retail_biomes_1_26_50.txt"
	v2193BiomeProjectionPath = "assets/biome-projection-v2193.json"
)

// v2193BiomesNewerThanPMMP are retail biomes the pinned PMMP 1.26.30 map
// predates; BDS and Dragonfly still cross-check them.
var v2193BiomesNewerThanPMMP = []string{"minecraft:dappled_forest"}

type v2193BiomeProjectionStats struct {
	IgnoredCount       int
	IgnoredFingerprint string
}

type v2193BiomeProjectionManifest struct {
	Schema      string                      `json:"schema"`
	GameVersion string                      `json:"game_version"`
	Protocol    uint32                      `json:"protocol"`
	Sources     v2193BiomeProjectionSources `json:"sources"`
	Allowlist   v2193BiomeProjectionAllow   `json:"allowlist"`
	Projection  v2193BiomeProjectionSummary `json:"projection"`
	Output      v2193BiomeProjectionOutput  `json:"output"`
}

type v2193BiomeProjectionSources struct {
	BDS       v2193BiomeBDSSource       `json:"bds"`
	PMMP      v2193BiomePMMPSource      `json:"pmmp"`
	Dragonfly v2193BiomeDragonflySource `json:"dragonfly"`
	Retail    v2193BiomeRetailSource    `json:"retail"`
}

type v2193BiomeBDSSource struct {
	ExecutableSHA256 string `json:"executable_sha256"`
	TableOffset      uint64 `json:"table_offset"`
	TableRecords     int    `json:"table_records"`
	TableSHA256      string `json:"table_sha256"`
	RetailRecords    int    `json:"retail_records"`
	RetailSHA256     string `json:"retail_sha256"`
}

type v2193BiomePMMPSource struct {
	Commit string `json:"commit"`
	SHA256 string `json:"sha256"`
}

type v2193BiomeDragonflySource struct {
	Module    string `json:"module"`
	Version   string `json:"version"`
	ModuleSum string `json:"module_sum"`
}

type v2193BiomeRetailSource struct {
	ArchiveSHA256    string `json:"bds_archive_sha256"`
	BinarySHA256     string `json:"bds_binary_sha256"`
	BiomeDefinitions int    `json:"biome_definitions"`
}

type v2193BiomeProjectionAllow struct {
	Path   string `json:"path"`
	SHA256 string `json:"sha256"`
	Count  int    `json:"count"`
}

type v2193BiomeProjectionSummary struct {
	Retained           int    `json:"retained"`
	IgnoredCount       int    `json:"ignored_count"`
	IgnoredFingerprint string `json:"ignored_fingerprint"`
}

type v2193BiomeProjectionOutput struct {
	Format string `json:"format"`
	Path   string `json:"path"`
	SHA256 string `json:"sha256"`
}

func verifyV2193FileSHA256(path, expected string) error {
	file, err := os.Open(path)
	if err != nil {
		return fmt.Errorf("open source: %w", err)
	}
	hash := sha256.New()
	_, copyErr := io.Copy(hash, file)
	closeErr := file.Close()
	if err := errors.Join(copyErr, closeErr); err != nil {
		return fmt.Errorf("hash source: %w", err)
	}
	actual := fmt.Sprintf("%x", hash.Sum(nil))
	if actual != expected {
		return fmt.Errorf("source SHA-256 %s does not match pinned identity", actual)
	}
	return nil
}

func readV2193BDSBiomeRecords(path string) ([]BiomeRecord, error) {
	if err := verifyV2193FileSHA256(path, v2193BDSExecutableSHA256); err != nil {
		return nil, err
	}
	file, err := os.Open(path)
	if err != nil {
		return nil, fmt.Errorf("open pinned BDS executable: %w", err)
	}
	defer file.Close()
	image, err := pe.NewFile(file)
	if err != nil {
		return nil, fmt.Errorf("parse pinned BDS PE: %w", err)
	}
	if image.Machine != pe.IMAGE_FILE_MACHINE_AMD64 {
		return nil, fmt.Errorf("pinned BDS PE machine %#x is not AMD64", image.Machine)
	}
	optional, ok := image.OptionalHeader.(*pe.OptionalHeader64)
	if !ok || optional.ImageBase != v2193ImageBase {
		return nil, errors.New("pinned BDS PE has an unexpected image base")
	}
	table := make([]byte, v2193BiomeTableBytes)
	if _, err := file.ReadAt(table, v2193BiomeTableOffset); err != nil {
		return nil, fmt.Errorf("read pinned BDS biome table: %w", err)
	}
	if digest := fmt.Sprintf("%x", sha256.Sum256(table)); digest != v2193BiomeTableSHA256 {
		return nil, fmt.Errorf("pinned BDS biome table SHA-256 %s does not match", digest)
	}
	if digest := fmt.Sprintf("%x", sha256.Sum256(table[:v2193BiomeRetailTableBytes])); digest != v2193RetailTableSHA256 {
		return nil, fmt.Errorf("pinned BDS retail biome table SHA-256 %s does not match", digest)
	}
	resolve := func(address, length uint64) ([]byte, error) {
		offset, err := peVirtualAddressToFileOffset(image, address, length)
		if err != nil {
			return nil, err
		}
		name := make([]byte, int(length))
		if _, err := file.ReadAt(name, int64(offset)); err != nil {
			return nil, err
		}
		return name, nil
	}
	return parseV2193BiomeRecords(table, resolve)
}

func peVirtualAddressToFileOffset(image *pe.File, address, length uint64) (uint64, error) {
	optional, ok := image.OptionalHeader.(*pe.OptionalHeader64)
	if !ok || address < optional.ImageBase {
		return 0, errors.New("virtual address is outside the PE image")
	}
	rva := address - optional.ImageBase
	for _, section := range image.Sections {
		start := uint64(section.VirtualAddress)
		rawSize := uint64(section.Size)
		if rva < start || rva-start > rawSize || length > rawSize-(rva-start) {
			continue
		}
		return uint64(section.Offset) + (rva - start), nil
	}
	return 0, errors.New("virtual address does not map to PE section data")
}

func parseV2193BiomeRecords(table []byte, resolve func(uint64, uint64) ([]byte, error)) ([]BiomeRecord, error) {
	if len(table) != v2193BiomeTableBytes {
		return nil, fmt.Errorf("v2193 biome table size %d does not match %d", len(table), v2193BiomeTableBytes)
	}
	records := make([]BiomeRecord, 0, v2193BiomeSourceCount)
	seenIDs := make(map[uint32]struct{}, v2193BiomeSourceCount)
	seenNames := make(map[string]struct{}, v2193BiomeSourceCount)
	for index := range v2193BiomeSourceCount {
		start := index * v2193BiomeRecordSize
		id64 := binary.LittleEndian.Uint64(table[start : start+8])
		address := binary.LittleEndian.Uint64(table[start+8 : start+16])
		length := binary.LittleEndian.Uint64(table[start+16 : start+24])
		if id64 > uint64(^uint16(0)) {
			return nil, fmt.Errorf("v2193 biome record %d ID %d is outside uint16", index, id64)
		}
		if length == 0 || length > maxBiomeNameBytes {
			return nil, fmt.Errorf("v2193 biome record %d name length %d is outside bounds", index, length)
		}
		nameBytes, err := resolve(address, length)
		if err != nil {
			return nil, fmt.Errorf("resolve name for v2193 biome record %d: %w", index, err)
		}
		if uint64(len(nameBytes)) != length || !utf8.Valid(nameBytes) {
			return nil, fmt.Errorf("v2193 biome record %d has a malformed name", index)
		}
		name := string(nameBytes)
		if !strings.HasPrefix(name, "minecraft:") {
			return nil, fmt.Errorf("v2193 biome record %d lacks the required namespace prefix", index)
		}
		id := uint32(id64)
		if _, exists := seenIDs[id]; exists {
			return nil, fmt.Errorf("duplicate biome ID %d", id)
		}
		if _, exists := seenNames[name]; exists {
			return nil, fmt.Errorf("duplicate biome name at record %d", index)
		}
		seenIDs[id], seenNames[name] = struct{}{}, struct{}{}
		records = append(records, BiomeRecord{ID: id, Name: name})
	}
	return records, nil
}

func parseV2193BiomeAllowlist(data []byte) (map[string]struct{}, error) {
	if digest := fmt.Sprintf("%x", sha256.Sum256(data)); digest != v2193BiomeAllowSHA256 {
		return nil, fmt.Errorf("v2193 biome allowlist SHA-256 %s does not match pinned identity", digest)
	}
	lines := strings.Split(strings.TrimSuffix(string(data), "\n"), "\n")
	if len(lines) != v2193RetailBiomeCount {
		return nil, fmt.Errorf("v2193 biome allowlist contains %d names, want %d", len(lines), v2193RetailBiomeCount)
	}
	allowed := make(map[string]struct{}, len(lines))
	previous := ""
	for index, raw := range lines {
		name := strings.TrimSuffix(raw, "\r")
		if !strings.HasPrefix(name, "minecraft:") || len(name) > maxBiomeNameBytes || (index != 0 && name <= previous) {
			return nil, fmt.Errorf("v2193 biome allowlist is invalid at entry %d", index)
		}
		allowed[name] = struct{}{}
		previous = name
	}
	return allowed, nil
}

func readV2193BiomeAllowlist(path string) (map[string]struct{}, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read v2193 biome allowlist: %w", err)
	}
	return parseV2193BiomeAllowlist(data)
}

func decodeV2193PMMPBiomeMap(data []byte) (map[string]uint32, error) {
	decoder := json.NewDecoder(bytes.NewReader(data))
	var raw map[string]uint64
	if err := decoder.Decode(&raw); err != nil {
		return nil, fmt.Errorf("decode PMMP biome map: %w", err)
	}
	var trailing any
	if err := decoder.Decode(&trailing); !errors.Is(err, io.EOF) {
		return nil, errors.New("PMMP biome map has trailing JSON")
	}
	if digest := fmt.Sprintf("%x", sha256.Sum256(data)); digest != v2193PMMPBiomeMapSHA256 {
		return nil, fmt.Errorf("PMMP biome map SHA-256 %s does not match pinned identity", digest)
	}
	if len(raw) != v2193PMMPBiomeCount {
		return nil, fmt.Errorf("PMMP biome map contains %d records, want %d", len(raw), v2193PMMPBiomeCount)
	}
	result := make(map[string]uint32, len(raw))
	for rawName, id := range raw {
		name := canonicalBiomeName(rawName)
		if !strings.HasPrefix(name, "minecraft:") || id > uint64(^uint16(0)) {
			return nil, errors.New("PMMP biome map contains an invalid record")
		}
		if _, exists := result[name]; exists {
			return nil, errors.New("PMMP biome map contains a duplicate canonical name")
		}
		result[name] = uint32(id)
	}
	return result, nil
}

func readV2193PMMPBiomeMap(path string) (map[string]uint32, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read PMMP biome map: %w", err)
	}
	return decodeV2193PMMPBiomeMap(data)
}

func v2193DragonflyBiomeMap() (map[string]uint32, error) {
	records, err := collectBiomes(world.Biomes())
	if err != nil {
		return nil, err
	}
	if len(records) != v2193RetailBiomeCount {
		return nil, fmt.Errorf("Dragonfly biome map contains %d records, want %d", len(records), v2193RetailBiomeCount)
	}
	result := make(map[string]uint32, len(records))
	for _, record := range records {
		if _, exists := result[record.Name]; exists {
			return nil, fmt.Errorf("Dragonfly biome map duplicates %q", record.Name)
		}
		result[record.Name] = record.ID
	}
	return result, nil
}

func projectV2193BiomeRecords(source []BiomeRecord, allowed map[string]struct{}, pmmp, dragonfly map[string]uint32) ([]BiomeRecord, v2193BiomeProjectionStats, error) {
	stats := v2193BiomeProjectionStats{}
	if len(source) != v2193BiomeSourceCount || len(allowed) != v2193RetailBiomeCount {
		return nil, stats, errors.New("v2193 biome source or allowlist count does not match the pinned scope")
	}
	retained := make(map[string]uint32, v2193RetailBiomeCount)
	ignored := make([]BiomeRecord, 0)
	for _, record := range source {
		if _, keep := allowed[record.Name]; keep {
			retained[record.Name] = record.ID
		} else {
			ignored = append(ignored, record)
		}
	}
	if len(retained) != v2193RetailBiomeCount {
		return nil, stats, fmt.Errorf("v2193 biome projection is missing %d retained names", v2193RetailBiomeCount-len(retained))
	}
	if len(ignored) != v2193BiomeSourceCount-v2193RetailBiomeCount {
		return nil, stats, fmt.Errorf("v2193 biome projection ignored %d records, want %d", len(ignored), v2193BiomeSourceCount-v2193RetailBiomeCount)
	}
	pmmpScope := make(map[string]uint32, len(retained))
	for name, id := range retained {
		if !slices.Contains(v2193BiomesNewerThanPMMP, name) {
			pmmpScope[name] = id
		}
	}
	if err := compareV2193BiomeMap("PMMP", pmmpScope, pmmp, v2193PMMPBiomeCount); err != nil {
		return nil, stats, err
	}
	if err := compareV2193BiomeMap("Dragonfly", retained, dragonfly, v2193RetailBiomeCount); err != nil {
		return nil, stats, err
	}
	projected := make([]BiomeRecord, 0, len(retained))
	for name, id := range retained {
		projected = append(projected, BiomeRecord{ID: id, Name: name})
	}
	sort.Slice(projected, func(i, j int) bool { return projected[i].ID < projected[j].ID })
	fingerprint := sha256.New()
	for _, record := range ignored {
		_ = binary.Write(fingerprint, binary.LittleEndian, record.ID)
		_ = binary.Write(fingerprint, binary.LittleEndian, uint16(len(record.Name)))
		_, _ = fingerprint.Write([]byte(record.Name))
	}
	stats.IgnoredCount = len(ignored)
	stats.IgnoredFingerprint = fmt.Sprintf("%x", fingerprint.Sum(nil))
	return projected, stats, nil
}

func compareV2193BiomeMap(label string, retained, comparison map[string]uint32, want int) error {
	if len(comparison) != want {
		return fmt.Errorf("%s biome map contains %d records, want %d", label, len(comparison), want)
	}
	if len(retained) != want {
		return fmt.Errorf("%s comparison scope holds %d retained records, want %d", label, len(retained), want)
	}
	for name, id := range retained {
		if other, exists := comparison[name]; !exists || other != id {
			return fmt.Errorf("%s biome map disagrees with retained record %q", label, name)
		}
	}
	return nil
}

func encodeV2193BiomeProjection(records []BiomeRecord, stats v2193BiomeProjectionStats) ([]byte, []byte, error) {
	carrier, err := encodeBiomeRegistry(records)
	if err != nil {
		return nil, nil, err
	}
	if len(records) != v2193RetailBiomeCount || stats.IgnoredCount != v2193BiomeSourceCount-v2193RetailBiomeCount || len(stats.IgnoredFingerprint) != 64 {
		return nil, nil, errors.New("v2193 biome projection metadata is incomplete")
	}
	carrierSHA := fmt.Sprintf("%x", sha256.Sum256(carrier))
	manifest := v2193BiomeProjectionManifest{
		Schema: "cinnabar.biome-projection.v2", GameVersion: "1.26.50", Protocol: 2193,
		Sources: v2193BiomeProjectionSources{
			BDS: v2193BiomeBDSSource{
				ExecutableSHA256: v2193BDSExecutableSHA256, TableOffset: v2193BiomeTableOffset,
				TableRecords: v2193BiomeSourceCount, TableSHA256: v2193BiomeTableSHA256,
				RetailRecords: v2193RetailBiomeCount, RetailSHA256: v2193RetailTableSHA256,
			},
			PMMP:      v2193BiomePMMPSource{Commit: "bdb44a48fb6beffb6e9f6864f06d2232eb62b6a3", SHA256: v2193PMMPBiomeMapSHA256},
			Dragonfly: v2193BiomeDragonflySource{Module: dragonflyModule, Version: dragonflyVersion, ModuleSum: dragonflyModuleSum},
			Retail: v2193BiomeRetailSource{
				ArchiveSHA256: v2193CaptureArchiveSHA256, BinarySHA256: v2193CaptureBinarySHA256,
				BiomeDefinitions: v2193RetailBiomeCount,
			},
		},
		Allowlist:  v2193BiomeProjectionAllow{Path: v2193BiomeAllowlistPath, SHA256: v2193BiomeAllowSHA256, Count: v2193RetailBiomeCount},
		Projection: v2193BiomeProjectionSummary{Retained: len(records), IgnoredCount: stats.IgnoredCount, IgnoredFingerprint: stats.IgnoredFingerprint},
		Output:     v2193BiomeProjectionOutput{Format: biomeRegistryHeader, Path: v2193BiomeOutputPath, SHA256: carrierSHA},
	}
	manifestBytes, err := json.MarshalIndent(manifest, "", "  ")
	if err != nil {
		return nil, nil, fmt.Errorf("encode v2193 biome manifest: %w", err)
	}
	return carrier, append(manifestBytes, '\n'), nil
}

func writeV2193BiomeProjection(executablePath, pmmpPath, allowlistPath, outputPath, manifestPath string) error {
	source, err := readV2193BDSBiomeRecords(executablePath)
	if err != nil {
		return err
	}
	allowed, err := readV2193BiomeAllowlist(allowlistPath)
	if err != nil {
		return err
	}
	pmmp, err := readV2193PMMPBiomeMap(pmmpPath)
	if err != nil {
		return err
	}
	dragonfly, err := v2193DragonflyBiomeMap()
	if err != nil {
		return err
	}
	projected, stats, err := projectV2193BiomeRecords(source, allowed, pmmp, dragonfly)
	if err != nil {
		return err
	}
	carrier, manifest, err := encodeV2193BiomeProjection(projected, stats)
	if err != nil {
		return err
	}
	for _, path := range []string{outputPath, manifestPath} {
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			return fmt.Errorf("create v2193 biome output directory: %w", err)
		}
	}
	if err := os.WriteFile(outputPath, carrier, 0o644); err != nil {
		return fmt.Errorf("write v2193 biome registry: %w", err)
	}
	shaPath := strings.TrimSuffix(outputPath, filepath.Ext(outputPath)) + ".sha256"
	if err := os.WriteFile(shaPath, []byte(fmt.Sprintf("%x\n", sha256.Sum256(carrier))), 0o644); err != nil {
		return fmt.Errorf("write v2193 biome checksum: %w", err)
	}
	if err := os.WriteFile(manifestPath, manifest, 0o644); err != nil {
		return fmt.Errorf("write v2193 biome manifest: %w", err)
	}
	return nil
}
