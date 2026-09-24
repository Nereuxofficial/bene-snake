use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct NonPushableQueue<T: Sized + Send + Sync> {
    items: Vec<UnsafeCell<MaybeUninit<T>>>,
    next: AtomicUsize,
}

// SAFETY: T is Send, and moving the queue transfers exclusive ownership of its entries.
unsafe impl<T: Sized + Send + Sync> Send for NonPushableQueue<T> {}

// SAFETY: The atomic cursor gives each pop a distinct index. Entries are never
// moved after construction, and Drop requires exclusive access to the queue.
unsafe impl<T: Sized + Send + Sync> Sync for NonPushableQueue<T> {}

impl<T: Sized + Send + Sync> NonPushableQueue<T> {
    pub fn pop_front(&self) -> Option<T> {
        let index = self
            .next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |index| {
                (index < self.items.len()).then_some(index + 1)
            })
            .ok()?;

        // SAFETY: This index was claimed exactly once, and the vector is not
        // modified after construction. Drop cannot run while this borrow exists.
        Some(unsafe { (*self.items[index].get()).assume_init_read() })
    }

    pub fn is_empty(&self) -> bool {
        self.next.load(Ordering::Relaxed) == self.items.len()
    }

    pub fn new_from_iterator(iter: impl Iterator<Item = T>) -> Self {
        Self {
            items: iter
                .map(|item| UnsafeCell::new(MaybeUninit::new(item)))
                .collect(),
            next: AtomicUsize::new(0),
        }
    }
}

impl<T: Sized + Send + Sync> Drop for NonPushableQueue<T> {
    fn drop(&mut self) {
        for item in &mut self.items[*self.next.get_mut()..] {
            // SAFETY: These entries were never claimed, so they are initialized
            // and have not been read. Drop has exclusive access to the queue.
            unsafe { item.get_mut().assume_init_drop() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn test_empty_queue() {
        let queue: NonPushableQueue<i32> = NonPushableQueue::new_from_iterator(std::iter::empty());
        assert_eq!(queue.pop_front(), None);
        assert_eq!(queue.pop_front(), None); // Should still be None
    }

    #[test]
    fn test_single_element() {
        let queue = NonPushableQueue::new_from_iterator(vec![42].into_iter());
        assert_eq!(queue.pop_front(), Some(42));
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn test_multiple_elements_fifo_order() {
        let queue = NonPushableQueue::new_from_iterator(vec![1, 2, 3, 4, 5].into_iter());
        assert_eq!(queue.pop_front(), Some(1));
        assert_eq!(queue.pop_front(), Some(2));
        assert_eq!(queue.pop_front(), Some(3));
        assert_eq!(queue.pop_front(), Some(4));
        assert_eq!(queue.pop_front(), Some(5));
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn test_with_strings() {
        let queue = NonPushableQueue::new_from_iterator(
            vec!["hello".to_string(), "world".to_string(), "test".to_string()].into_iter(),
        );
        assert_eq!(queue.pop_front(), Some("hello".to_string()));
        assert_eq!(queue.pop_front(), Some("world".to_string()));
        assert_eq!(queue.pop_front(), Some("test".to_string()));
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn test_with_large_dataset() {
        let size = 1000;
        let data: Vec<usize> = (0..size).collect();
        let queue = NonPushableQueue::new_from_iterator(data.into_iter());

        for i in 0..size {
            assert_eq!(queue.pop_front(), Some(i));
        }
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn test_interleaved_pops() {
        let queue = NonPushableQueue::new_from_iterator(vec![1, 2, 3, 4].into_iter());
        assert_eq!(queue.pop_front(), Some(1));
        assert_eq!(queue.pop_front(), Some(2));
        assert_eq!(queue.pop_front(), Some(3));
        assert_eq!(queue.pop_front(), Some(4));
        assert_eq!(queue.pop_front(), None);
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn test_from_range_iterator() {
        let queue = NonPushableQueue::new_from_iterator(0..10);
        for i in 0..10 {
            assert_eq!(queue.pop_front(), Some(i));
        }
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn test_with_complex_types() {
        #[derive(Debug, PartialEq, Clone)]
        struct ComplexType {
            id: u32,
            name: String,
            values: Vec<i32>,
        }

        let items = vec![
            ComplexType {
                id: 1,
                name: "first".to_string(),
                values: vec![1, 2, 3],
            },
            ComplexType {
                id: 2,
                name: "second".to_string(),
                values: vec![4, 5, 6],
            },
        ];

        let queue = NonPushableQueue::new_from_iterator(items.clone().into_iter());
        assert_eq!(queue.pop_front(), Some(items[0].clone()));
        assert_eq!(queue.pop_front(), Some(items[1].clone()));
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn test_concurrent_single_consumer() {
        let queue = Arc::new(NonPushableQueue::new_from_iterator(0..100));
        let queue_clone = Arc::clone(&queue);

        let handle = thread::spawn(move || {
            let mut count = 0;
            while queue_clone.pop_front().is_some() {
                count += 1;
            }
            count
        });

        let result = handle.join().unwrap();
        assert_eq!(result, 100);
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn test_concurrent_multiple_consumers() {
        let queue = Arc::new(NonPushableQueue::new_from_iterator(0..1000));
        let num_threads = 4;
        let barrier = Arc::new(Barrier::new(num_threads));

        let mut handles = vec![];

        for _ in 0..num_threads {
            let queue_clone = Arc::clone(&queue);
            let barrier_clone = Arc::clone(&barrier);

            let handle = thread::spawn(move || {
                barrier_clone.wait(); // Ensure all threads start at the same time
                let mut items = vec![];
                while let Some(item) = queue_clone.pop_front() {
                    items.push(item);
                }
                items
            });

            handles.push(handle);
        }

        let mut all_items = vec![];
        for handle in handles {
            let items = handle.join().unwrap();
            all_items.extend(items);
        }

        // Check that all items were popped exactly once
        all_items.sort_unstable();
        let expected: Vec<i32> = (0..1000).collect();
        assert_eq!(all_items, expected);
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn test_concurrent_high_contention() {
        let queue = Arc::new(NonPushableQueue::new_from_iterator(0..10000));
        let num_threads = 8;
        let barrier = Arc::new(Barrier::new(num_threads));

        let mut handles = vec![];

        for _ in 0..num_threads {
            let queue_clone = Arc::clone(&queue);
            let barrier_clone = Arc::clone(&barrier);

            let handle = thread::spawn(move || {
                barrier_clone.wait();
                let mut count = 0;
                while queue_clone.pop_front().is_some() {
                    count += 1;
                }
                count
            });

            handles.push(handle);
        }

        let total: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();

        assert_eq!(total, 10000);
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn test_no_duplicate_items_concurrent() {
        let queue = Arc::new(NonPushableQueue::new_from_iterator(0..5000));
        let num_threads = 10;

        let mut handles = vec![];

        for _ in 0..num_threads {
            let queue_clone = Arc::clone(&queue);

            let handle = thread::spawn(move || {
                let mut items = HashSet::new();
                while let Some(item) = queue_clone.pop_front() {
                    // If insert returns false, the item was already in the set
                    assert!(items.insert(item), "Duplicate item found!");
                }
                items
            });

            handles.push(handle);
        }

        let mut all_items = HashSet::new();
        for handle in handles {
            let items = handle.join().unwrap();
            for item in items {
                assert!(all_items.insert(item), "Item appeared in multiple threads!");
            }
        }

        assert_eq!(all_items.len(), 5000);
        for i in 0..5000 {
            assert!(all_items.contains(&i));
        }
    }

    #[test]
    fn test_memory_safety_after_exhaustion() {
        let queue = Arc::new(NonPushableQueue::new_from_iterator(
            vec![1, 2, 3].into_iter(),
        ));

        // Exhaust the queue
        while queue.pop_front().is_some() {}

        // Spawn multiple threads trying to pop from empty queue
        let mut handles = vec![];
        for _ in 0..10 {
            let queue_clone = Arc::clone(&queue);
            let handle = thread::spawn(move || {
                for _ in 0..100 {
                    assert_eq!(queue_clone.pop_front(), None);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_with_zero_sized_types() {
        #[derive(Debug, PartialEq)]
        struct ZeroSized;

        let queue =
            NonPushableQueue::new_from_iterator(vec![ZeroSized, ZeroSized, ZeroSized].into_iter());

        assert!(queue.pop_front().is_some());
        assert!(queue.pop_front().is_some());
        assert!(queue.pop_front().is_some());
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn test_drop_is_called() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

        struct DropCounter;

        impl Drop for DropCounter {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::SeqCst);
            }
        }

        DROP_COUNT.store(0, Ordering::SeqCst);

        {
            let queue = NonPushableQueue::new_from_iterator(
                vec![DropCounter, DropCounter, DropCounter].into_iter(),
            );

            queue.pop_front(); // Drop 1
            queue.pop_front(); // Drop 2
            // Third element still in queue
        } // Queue dropped here, should drop remaining element

        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_concurrent_stress_test() {
        for _ in 0..10 {
            let queue = Arc::new(NonPushableQueue::new_from_iterator(0..100));
            let mut handles = vec![];

            for _ in 0..4 {
                let queue_clone = Arc::clone(&queue);
                let handle = thread::spawn(move || {
                    let mut local_count = 0;
                    while queue_clone.pop_front().is_some() {
                        local_count += 1;
                        // Small yield to increase contention
                        thread::yield_now();
                    }
                    local_count
                });
                handles.push(handle);
            }

            let total: i32 = handles.into_iter().map(|h| h.join().unwrap()).sum();

            assert_eq!(total, 100);
        }
    }

    #[test]
    fn test_iterator_from_filter() {
        let queue = NonPushableQueue::new_from_iterator((0..20).filter(|x| x % 2 == 0));

        for i in 0..10 {
            assert_eq!(queue.pop_front(), Some(i * 2));
        }
        assert_eq!(queue.pop_front(), None);
    }

    #[test]
    fn test_iterator_from_map() {
        let queue = NonPushableQueue::new_from_iterator((0..5).map(|x| x * 10));

        assert_eq!(queue.pop_front(), Some(0));
        assert_eq!(queue.pop_front(), Some(10));
        assert_eq!(queue.pop_front(), Some(20));
        assert_eq!(queue.pop_front(), Some(30));
        assert_eq!(queue.pop_front(), Some(40));
        assert_eq!(queue.pop_front(), None);
    }
}
