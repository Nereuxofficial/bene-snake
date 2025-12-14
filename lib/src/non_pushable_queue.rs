use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicPtr, Ordering};

pub struct NonPushableQueue<T: Sized + Send + Sync> {
    head: AtomicPtr<Node<T>>,
    original_head: *mut Node<T>, // For cleanup in Drop
}

pub struct Node<T: Sized + Send + Sync> {
    data: MaybeUninit<T>,
    next: AtomicPtr<Node<T>>,
}

// Pointer tagging: lowest bit = 1 means data has been consumed
const CONSUMED_BIT: usize = 0x1;

#[inline]
fn mark_consumed<T: Sized + Send + Sync>(ptr: *mut Node<T>) -> *mut Node<T> {
    ((ptr as usize) | CONSUMED_BIT) as *mut _
}

#[inline]
fn is_consumed<T: Sized + Send + Sync>(ptr: *mut Node<T>) -> bool {
    (ptr as usize) & CONSUMED_BIT != 0
}

#[inline]
fn clear_consumed_bit<T: Sized + Send + Sync>(ptr: *mut Node<T>) -> *mut Node<T> {
    ((ptr as usize) & !CONSUMED_BIT) as *mut _
}

// SAFETY: NonPushableQueue is safe to send between threads because:
// - The original_head pointer is only used in Drop, which has &mut self (exclusive access)
// - The head AtomicPtr handles synchronization for concurrent access
// - All actual data access goes through proper atomic operations
unsafe impl<T: Sized + Send + Sync> Send for NonPushableQueue<T> {}

// SAFETY: NonPushableQueue is safe to share between threads because:
// - pop_front uses atomic operations for all shared state
// - original_head is only read in Drop which requires exclusive access
unsafe impl<T: Sized + Send + Sync> Sync for NonPushableQueue<T> {}

impl<T: Sized + Send + Sync> NonPushableQueue<T> {
    pub fn pop_front(&self) -> Option<T> {
        loop {
            // Load the current head
            let current = self.head.load(Ordering::Acquire);

            // If null, queue is empty
            if current.is_null() {
                return None;
            }

            // Get the next node (the node that will become the new head)
            // SAFETY: We haven't freed this node yet (nodes are only freed in Drop),
            // so it's safe to read from it even if another thread is modifying it
            let next_raw = unsafe { (*current).next.load(Ordering::Acquire) };

            // Check if this node has already been consumed by another thread
            if is_consumed(next_raw) {
                // Node already consumed, move to the actual next node
                let next = clear_consumed_bit(next_raw);
                // Try to help by advancing the head pointer
                let _ =
                    self.head
                        .compare_exchange(current, next, Ordering::Release, Ordering::Acquire);
                continue;
            }

            let next = next_raw;

            // Try to atomically swap the head from current to next
            // This ensures only one thread can claim this node
            if self
                .head
                .compare_exchange(current, next, Ordering::Release, Ordering::Acquire)
                .is_ok()
            {
                // We successfully claimed this node
                // Mark it as consumed by setting the tag bit in the next pointer
                unsafe {
                    let next_tagged = mark_consumed(next);
                    (*current).next.store(next_tagged, Ordering::Release);

                    // SAFETY: We've successfully moved the head pointer past this node,
                    // so we have exclusive access to take its data
                    let data = (*current).data.assume_init_read();
                    return Some(data);
                }
            }
            // If CAS failed, another thread claimed it, try again
        }
    }

    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Acquire).is_null()
    }

    pub fn new_from_iterator(iter: impl Iterator<Item = T>) -> Self {
        let mut head_ptr: *mut Node<T> = std::ptr::null_mut();
        let mut tail_ptr: *mut Node<T> = std::ptr::null_mut();

        for item in iter {
            let new_node = Box::new(Node {
                data: MaybeUninit::new(item),
                next: AtomicPtr::new(std::ptr::null_mut()),
            });
            let new_node_ptr = Box::into_raw(new_node);

            if tail_ptr.is_null() {
                // First node
                head_ptr = new_node_ptr;
                tail_ptr = new_node_ptr;
            } else {
                // Append to tail
                unsafe {
                    (*tail_ptr).next.store(new_node_ptr, Ordering::Relaxed);
                }
                tail_ptr = new_node_ptr;
            }
        }

        Self {
            head: AtomicPtr::new(head_ptr),
            original_head: head_ptr,
        }
    }
}

impl<T: Sized + Send + Sync> Drop for NonPushableQueue<T> {
    fn drop(&mut self) {
        // Free all nodes starting from the original head (including those already popped)
        let mut current = self.original_head;
        while !current.is_null() {
            unsafe {
                let mut node = Box::from_raw(current);
                let next_raw = *node.next.get_mut();

                // Check if data was consumed - if not, we need to drop it
                if !is_consumed(next_raw) {
                    // Data was never consumed, drop it manually
                    node.data.assume_init_drop();
                }

                // Move to next node (clear the consumed bit if present)
                current = clear_consumed_bit(next_raw);
                // node is dropped here
            }
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
