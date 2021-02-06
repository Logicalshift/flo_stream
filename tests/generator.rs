use flo_stream::*;

use futures::prelude::*;
use futures::executor;
use futures::channel::mpsc;

#[test]
fn number_generator() {
    executor::block_on(async {
        // Create a simple generator stream that adds one to the numbers it receives (this is the same as 'map()' except using a generator)
        let mut generated_stream = generator_stream(move |yield_value| async move {
            for num in 0u32..3 {
                yield_value(num).await;
            }
        });

        // Just generates 0, 1, 2
        assert!(generated_stream.next().await == Some(0));
        assert!(generated_stream.next().await == Some(1));
        assert!(generated_stream.next().await == Some(2));

        // Should end after 3 numbers
        assert!(generated_stream.next().await == None);
    })
}
#[test]
fn add_one_generator() {
    executor::block_on(async {
        let (mut numbers_in, mut numbers_out) = mpsc::channel(20);

        // Create a simple generator stream that adds one to the numbers it receives (this is the same as 'map()' except using a generator)
        let mut generated_stream = generator_stream(move |yield_value| async move {
            for _ in 0u32..3 {
                // Read the next number
                let next_number: Option<i32> = numbers_out.next().await;

                // Yield the next number + 3
                if let Some(next_number) = next_number {
                    yield_value(next_number + 3).await;
                } else {
                    break;
                }
            }
        });

        // Send some numbers and retrieve the results
        numbers_in.send(1).await.unwrap();
        assert!(generated_stream.next().await == Some(4));

        numbers_in.send(5).await.unwrap();
        assert!(generated_stream.next().await == Some(8));

        numbers_in.send(39).await.unwrap();
        assert!(generated_stream.next().await == Some(42));

        // Should end after 3 numbers
        assert!(generated_stream.next().await == None);
    })
}