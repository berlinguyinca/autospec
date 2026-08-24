package main

// GPU telemetry, volunteered.
//
// The node never asks for this and cannot: nothing in the protocol lets it tell
// an agent what to run. The agent offers what it knows about its own box, which
// is the same direction of trust as the model list -- and the same reason the
// target address lives in this file's sibling rather than arriving over the wire.
//
// nvidia-smi is EXEC'd, not linked, so this stays a standard-library binary with
// no dependencies. A box without it -- a Mac, a CPU node, a machine with a
// different vendor's cards -- reports nothing and says so by OMISSION rather than
// by failing: the operator's written description in the config still stands, and
// a panel that showed an error where a card should be would be worse than a
// panel that shows the description.

import (
	"context"
	"os/exec"
	"strconv"
	"strings"
	"time"
)

// One card, in the same shape the node's own nvidia-smi collector emits, so the
// dashboard renders a remote card with the code that already draws a local one.
type card struct {
	Index    int     `json:"index"`
	Name     string  `json:"name"`
	MemTotal int     `json:"mem_total_mib"`
	MemUsed  int     `json:"mem_used_mib"`
	Util     int     `json:"util_pct"`
	Temp     int     `json:"temp_c"`
	Power    float64 `json:"power_w"`
}

type capabilities struct {
	Type  string `json:"type"`
	Cards []card `json:"cards"`
}

const smiFields = "index,name,memory.total,memory.used,utilization.gpu," +
	"temperature.gpu,power.draw"

// capabilityInterval is comfortably slower than the dashboard's own refresh: this
// is telemetry about someone else's box, and forking nvidia-smi is not free.
const capabilityInterval = 20 * time.Second

func readCards() []card {
	// Bounded: nvidia-smi can hang on a wedged driver, and a telemetry fork must
	// never be able to stall the loop that keeps this server registered.
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	out, err := exec.CommandContext(ctx, "nvidia-smi",
		"--query-gpu="+smiFields, "--format=csv,noheader,nounits").Output()
	if err != nil {
		return nil
	}
	var cards []card
	for _, line := range strings.Split(strings.TrimSpace(string(out)), "\n") {
		f := strings.Split(line, ",")
		if len(f) < 7 {
			continue
		}
		for i := range f {
			f[i] = strings.TrimSpace(f[i])
		}
		// A field nvidia-smi could not read comes back as "[N/A]" rather than a
		// number -- on a card that does not report power, for instance. Those
		// parse to zero and the card is still reported: one missing reading is
		// not a reason to hide a GPU.
		cards = append(cards, card{
			Index: atoi(f[0]), Name: f[1],
			MemTotal: atoi(f[2]), MemUsed: atoi(f[3]),
			Util: atoi(f[4]), Temp: atoi(f[5]), Power: atof(f[6]),
		})
	}
	return cards
}

func atoi(s string) int {
	n, err := strconv.Atoi(s)
	if err != nil {
		return 0
	}
	return n
}

func atof(s string) float64 {
	f, err := strconv.ParseFloat(s, 64)
	if err != nil {
		return 0
	}
	return f
}
