// sample.go — Go fixture for autospec-docs walker unit tests.

package main

import (
	"encoding/json"
	"fmt"
	"net/http"
)

const DefaultPort = 8080

type Config struct {
	Host string `json:"host"`
	Port int    `json:"port"`
}

func ParseConfig(data []byte) (Config, error) {
	var cfg Config
	err := json.Unmarshal(data, &cfg)
	return cfg, err
}

func FormatAddress(cfg Config) string {
	return fmt.Sprintf("%s:%d", cfg.Host, cfg.Port)
}

func main() {
	mux := http.NewServeMux()
	mux.HandleFunc("/health", func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
		fmt.Fprintln(w, "ok")
	})
	addr := fmt.Sprintf(":%d", DefaultPort)
	http.ListenAndServe(addr, mux)
}
