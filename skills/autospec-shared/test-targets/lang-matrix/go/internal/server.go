// server.go — Gin HTTP server
package internal

import (
	"net/http"

	"github.com/gin-gonic/gin"
)

// StartServer starts the HTTP server on addr.
func StartServer(addr string) error {
	r := gin.Default()
	r.GET("/greet/:name", func(c *gin.Context) {
		name := c.Param("name")
		c.JSON(http.StatusOK, gin.H{"message": "Hello, " + name + "!"})
	})
	return r.Run(addr)
}
