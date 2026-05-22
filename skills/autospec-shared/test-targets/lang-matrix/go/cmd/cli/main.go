// main.go — CLI entry point using cobra
package main

import (
	"fmt"
	"os"

	"github.com/spf13/cobra"
)

func main() {
	rootCmd := &cobra.Command{
		Use:   "hello <name>",
		Short: "CLI greeter",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			fmt.Println(greet(args[0]))
		},
	}
	if err := rootCmd.Execute(); err != nil {
		os.Exit(1)
	}
}

func greet(name string) string {
	return "Hello, " + name + "!"
}
