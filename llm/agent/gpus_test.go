package main

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestHardwareFingerprintIgnoresVolatileTelemetryAndIndex(t *testing.T) {
	a := []card{{Index: 0, Name: "RTX 6000", MemTotal: 97887, MemUsed: 10, Util: 2, Temp: 30, Power: 100}}
	b := []card{{Index: 7, Name: "RTX 6000", MemTotal: 97887, MemUsed: 90000, Util: 99, Temp: 90, Power: 300}}
	if hardwareFingerprint(a) != hardwareFingerprint(b) {
		t.Fatal("volatile telemetry changed fingerprint")
	}
}

func TestCapabilitiesOmitsEmptyHardwareFingerprint(t *testing.T) {
	encoded, err := json.Marshal(capabilities{Type: "capabilities", Cards: nil, HardwareFingerprint: ""})
	if err != nil {
		t.Fatalf("marshal capabilities: %v", err)
	}
	if strings.Contains(string(encoded), "hardware_fingerprint") {
		t.Fatalf("empty hardware fingerprint was serialized: %s", encoded)
	}
}

func TestHardwareFingerprintChangesWithCardClassCapacityOrCount(t *testing.T) {
	one := []card{{Name: "RTX 6000", MemTotal: 97887}}
	two := append(append([]card{}, one...), card{Name: "RTX 6000", MemTotal: 97887})
	other := []card{{Name: "A100", MemTotal: 81920}}
	if hardwareFingerprint(one) == hardwareFingerprint(two) {
		t.Fatal("count was ignored")
	}
	if hardwareFingerprint(one) == hardwareFingerprint(other) {
		t.Fatal("hardware change was ignored")
	}
}

func TestHardwareFingerprintOmitsAnEmptyObservation(t *testing.T) {
	if hardwareFingerprint(nil) != "" {
		t.Fatal("a failed GPU probe became a hardware generation")
	}
}

func TestHardwareFingerprintIsIndependentOfCardOrder(t *testing.T) {
	a := []card{{Name: "A100", MemTotal: 81920}, {Name: "L40S", MemTotal: 46068}}
	b := []card{a[1], a[0]}
	if hardwareFingerprint(a) != hardwareFingerprint(b) {
		t.Fatal("card order changed generation")
	}
}
