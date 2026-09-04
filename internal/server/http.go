package server

import (
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/aleksandrgradoboev-svg/gf-data-1c/internal/journal"
)

// HTTPOptions — параметры сетевого режима.
type HTTPOptions struct {
	// Addr — что слушать. Умолчание намеренно локальное: сервер отдаёт данные
	// информационных баз, и «слушать всё подряд» должно быть осознанным решением.
	Addr string
	// Token — если задан, каждый запрос обязан нести его в заголовке Authorization
	// как Bearer. Пусто — проверки нет.
	Token string
}

// DefaultHTTPAddr — адрес по умолчанию для сетевого режима.
const DefaultHTTPAddr = "127.0.0.1:9077"

// ServeHTTP поднимает сетевой сервер MCP.
//
// Транспорт streamable: один адрес обслуживает несколько независимых сессий, поэтому
// агент может работать в нескольких сеансах сразу — каждый со своей историей, но с
// общим реестром баз и одним процессом на машине.
func ServeHTTP(opts Options, httpOpts HTTPOptions) error {
	return ServeHTTPListener(opts, httpOpts, nil)
}

// ServeHTTPListener делает то же, но сообщает фактический адрес прослушивания в канал.
//
// Нужно там, где порт выбирает операционная система (addr вида «127.0.0.1:0»): иначе
// вызывающему неоткуда узнать, куда подключаться.
func ServeHTTPListener(opts Options, httpOpts HTTPOptions, announce chan<- string) error {
	addr := strings.TrimSpace(httpOpts.Addr)
	if addr == "" {
		addr = DefaultHTTPAddr
	}

	// Сервер собирается заново на каждую сессию: состояние (реестр, таймауты) общее,
	// а сессионные данные SDK держит отдельно.
	handler := mcp.NewStreamableHTTPHandler(
		func(*http.Request) *mcp.Server { return New(opts) },
		nil,
	)

	mux := http.NewServeMux()
	mux.Handle("/mcp", authorize(httpOpts.Token, handler))
	mux.HandleFunc("/health", func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprintf(w, "gf-data-1c %s\n", Version)
	})

	listener, err := net.Listen("tcp", addr)
	if err != nil {
		return fmt.Errorf("адрес %s не занят сервером: %w", addr, err)
	}

	if announce != nil {
		announce <- listener.Addr().String()
	}

	fmt.Fprintf(sink(), "gf-data-1c %s слушает http://%s/mcp\n", Version, listener.Addr())
	if !isLoopback(listener.Addr()) {
		fmt.Fprintf(sink(), "ВНИМАНИЕ: адрес не локальный — сервер отдаёт данные баз всем, "+
			"кто до него дотянется.%s\n", tokenAdvice(httpOpts.Token))
	}
	journal.Writef("сетевой режим: слушаю %s", listener.Addr())

	return http.Serve(listener, mux)
}

// authorize проверяет токен, если он задан.
func authorize(token string, next http.Handler) http.Handler {
	if token == "" {
		return next
	}
	want := "Bearer " + token
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != want {
			journal.Writef("сетевой режим: отказ в доступе %s", r.RemoteAddr)
			http.Error(w, "нужен заголовок Authorization: Bearer <токен>", http.StatusUnauthorized)
			return
		}
		next.ServeHTTP(w, r)
	})
}

func isLoopback(addr net.Addr) bool {
	host, _, err := net.SplitHostPort(addr.String())
	if err != nil {
		return false
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

func tokenAdvice(token string) string {
	if token != "" {
		return " Токен задан."
	}
	return " Токен НЕ задан — поставьте -token."
}

// sink — куда писать служебные сообщения сетевого режима.
//
// В stdio-режиме печать в стандартный вывод сломала бы протокол, поэтому она вынесена
// в отдельную функцию: здесь вывод свободен, но правило остаётся видимым.
func sink() io.Writer { return os.Stderr }
