package fetch

import (
	"sync"
	"time"
)

// ttlCache is a small in-memory cache of fetch results. It exists to make
// repeated agent reads of the same URL instant and to spare upstream servers;
// the durable corpus lives in the index, not here.
type ttlCache struct {
	mu  sync.Mutex
	ttl time.Duration
	m   map[string]entry
}

type entry struct {
	value Result
	at    time.Time
}

func newTTLCache(ttl time.Duration) *ttlCache {
	if ttl <= 0 {
		ttl = 10 * time.Minute
	}
	return &ttlCache{ttl: ttl, m: map[string]entry{}}
}

// Get returns a cached result if it is within the TTL.
func (c *ttlCache) Get(key string) (Result, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	e, ok := c.m[key]
	if !ok || time.Since(e.at) > c.ttl {
		if ok {
			delete(c.m, key)
		}
		return Result{}, false
	}
	return e.value, true
}

// Put stores a result. Errors are not cached, so a transient failure is retried.
func (c *ttlCache) Put(key string, value Result) {
	if value.Error != "" {
		return
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	// Bound the cache so a long-lived process cannot grow without limit.
	if len(c.m) > 2048 {
		for k := range c.m {
			delete(c.m, k)
			if len(c.m) <= 1024 {
				break
			}
		}
	}
	c.m[key] = entry{value: value, at: time.Now()}
}
